#![cfg(windows)]

#[path = "../src/op/job.rs"]
#[allow(dead_code)]
mod op_job;
#[path = "support/process.rs"]
mod process;

use std::{
    ffi::c_void,
    fs,
    io::{BufRead, BufReader},
    os::windows::{
        io::{AsRawHandle, IntoRawHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

use process::{Sandbox, resolve_tool};
use serde_json::json;

const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_SUSPENDED: u32 = 0x0000_0004;
const CTRL_BREAK_EVENT: u32 = 1;
static CONSOLE: Mutex<()> = Mutex::new(());

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GenerateConsoleCtrlEvent(event: u32, process_group: u32) -> i32;
    fn GetConsoleCP() -> u32;
    fn AllocConsole() -> i32;
    fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
    fn Thread32First(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
    fn Thread32Next(snapshot: *mut c_void, entry: *mut ThreadEntry) -> i32;
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> *mut c_void;
    fn ResumeThread(thread: *mut c_void) -> u32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn WaitForSingleObject(handle: *mut c_void, millis: u32) -> u32;
}

fn console() -> std::sync::MutexGuard<'static, ()> {
    let guard = CONSOLE.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: scalar Win32 calls; a console is needed for targeted control events.
    unsafe {
        if GetConsoleCP() == 0 {
            assert_ne!(AllocConsole(), 0, "{}", std::io::Error::last_os_error());
        }
    }
    guard
}

fn send_break(pid: u32) {
    // SAFETY: pid is a live child started as its own console process group.
    assert_ne!(
        unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
}

fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "child did not reach the expected state"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Handle(*mut c_void);
impl Handle {
    fn new_job() -> Self {
        Self(
            op_job::create_job()
                .expect("create cleanup Job")
                .into_raw_handle(),
        )
    }

    fn close(&mut self) {
        if !self.0.is_null() && self.0 as isize != -1 {
            // SAFETY: this value owns the handle, and closes it only once.
            unsafe {
                CloseHandle(self.0);
            }
            self.0 = std::ptr::null_mut();
        }
    }
}

const OP_ARGS: &[&str] = &[
    "item", "get", "cfg", "--vault", "vault", "--format", "json", "--reveal",
];

fn catching_op_fixture(s: &Sandbox) -> PathBuf {
    let ready = s.root.join("op-ready");
    s.owned(&ready);
    s.fake_op(&json!({"responses":[{
        "argv":OP_ARGS,"delayMs":30000,"stdout":"{}",
        "catchCtrlBreak":{"readyFile":ready}
    }]}));
    ready
}

fn ready_op(ready: &Path) -> Handle {
    let mut pid = None;
    until(|| {
        pid = fs::read_to_string(ready)
            .ok()
            .and_then(|s| s.parse::<u32>().ok());
        pid.is_some()
    });
    // SAFETY: retain SYNCHRONIZE access to this live fixture process, even after exit.
    let handle = Handle(unsafe { OpenProcess(0x0010_0000, 0, pid.unwrap()) });
    assert!(!handle.0.is_null(), "{}", std::io::Error::last_os_error());
    handle
}

#[test]
fn fake_op_catches_ctrl_break_and_exits_one_with_stderr() {
    let _console = console();
    let s = Sandbox::new();
    let ready = catching_op_fixture(&s);
    let mut command = s.op_command(s.tools.join("op.exe"));
    command
        .args(OP_ARGS)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Guard::spawn(&mut command);
    let _op = ready_op(&ready);
    send_break(child.0.as_ref().unwrap().id());
    until(|| child.0.as_mut().unwrap().try_wait().unwrap().is_some());
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"fake-op: caught Ctrl-Break\n");
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.close();
    }
}

#[repr(C)]
#[derive(Default)]
struct ThreadEntry {
    size: u32,
    usage: u32,
    tid: u32,
    owner_pid: u32,
    base_priority: i32,
    delta_priority: i32,
    flags: u32,
}

fn resume_child(child: &Child) {
    // SAFETY: TH32CS_SNAPTHREAD snapshots thread metadata; it runs no child code.
    let snapshot = Handle(unsafe { CreateToolhelp32Snapshot(4, 0) });
    assert_ne!(
        snapshot.0 as isize,
        -1,
        "{}",
        std::io::Error::last_os_error()
    );
    let mut entry = ThreadEntry {
        size: std::mem::size_of::<ThreadEntry>() as u32,
        ..Default::default()
    };
    // SAFETY: the snapshot is live; entry has the required repr(C) layout and size.
    let mut found = unsafe { Thread32First(snapshot.0, &mut entry) };
    while found != 0 {
        if entry.owner_pid == child.id() {
            // SAFETY: request only THREAD_SUSPEND_RESUME for this child's startup thread.
            let thread = Handle(unsafe { OpenThread(2, 0, entry.tid) });
            assert!(!thread.0.is_null(), "{}", std::io::Error::last_os_error());
            // SAFETY: the child remains suspended until after assignment to its job.
            assert_ne!(
                unsafe { ResumeThread(thread.0) },
                u32::MAX,
                "{}",
                std::io::Error::last_os_error()
            );
            return;
        }
        // SAFETY: the same live snapshot and entry buffer are reused.
        found = unsafe { Thread32Next(snapshot.0, &mut entry) };
    }
    panic!("suspended child startup thread not found");
}

struct Guard(Option<Child>, Handle);
impl Guard {
    fn spawn(command: &mut Command) -> Self {
        let job = Handle::new_job();
        // Assign before running any child code, so no grandchild can escape the job.
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED);
        let guard = Self(Some(command.spawn().unwrap()), job);
        let handle = guard.0.as_ref().unwrap().as_raw_handle();
        // SAFETY: both handles are live and the child is still suspended.
        assert_ne!(
            unsafe { AssignProcessToJobObject(guard.1.0, handle) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        resume_child(guard.0.as_ref().unwrap());
        guard
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.1.close();
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn ctrl_break_during_op_exits_130_without_cancellation_output() {
    let _console = console();
    let s = Sandbox::new();
    let ready = catching_op_fixture(&s);
    let mut command = s.cli();
    command
        .args([
            "list",
            "targets",
            "--vault",
            "vault",
            "--config-item",
            "cfg",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Guard::spawn(&mut command);
    let op = ready_op(&ready);
    // Progress precedes op; cancellation itself must produce no output.
    let mut stderr = BufReader::new(child.0.as_mut().unwrap().stderr.take().unwrap());
    let mut progress = String::new();
    stderr.read_line(&mut progress).unwrap();
    assert_eq!(progress, "envswitch [info] reading config item cfg\n");
    send_break(child.0.as_ref().unwrap().id());
    until(|| child.0.as_mut().unwrap().try_wait().unwrap().is_some());
    // The product Job must kill op before the test's outer cleanup Job closes.
    until(|| unsafe { WaitForSingleObject(op.0, 0) } == 0);
    let mut child = child.0.take().unwrap();
    child.stderr = Some(stderr.into_inner());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130));
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

/// The fixture accepts the ACL preflight, then waits for the test to release it.
/// Its Ctrl-Break handler proves the event reached the inherited console group.
const SLOW_INSTALLER: &str = r#"
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
public class Installer {
    delegate bool Handler(uint kind);
    [DllImport("kernel32.dll")] static extern bool SetConsoleCtrlHandler(Handler callback, bool add);
    static Handler callback = Break;
    static string Root { get { return Environment.GetEnvironmentVariable("ENVSWITCH_SIGNAL_ROOT"); } }
    static bool Break(uint kind) {
        if (kind != 1) return false;
        File.WriteAllText(Path.Combine(Root,"handled"), "break");
        return true;
    }
    public static int Main(string[] args) {
        if (Array.IndexOf(args,"-Command") >= 0) return 0;
        if (Array.IndexOf(args,"-File") < 0) return 96;
        if (!SetConsoleCtrlHandler(callback,true)) return 95;
        File.WriteAllText(Path.Combine(Root,"ready"), "ready");
        var deadline = DateTime.UtcNow.AddSeconds(15);
        while (!File.Exists(Path.Combine(Root,"release"))) {
            if (DateTime.UtcNow > deadline) return 94;
            Thread.Sleep(10);
        }
        Console.WriteLine("installer cleanup complete");
        return 7;
    }
}
"#;

#[test]
fn ctrl_break_during_update_waits_for_installer_and_returns_its_status() {
    let _console = console();
    let s = Sandbox::new();
    let source = s.root.join("installer.cs");
    let executable = s.root.join("slow-installer.exe");
    s.write(&source, SLOW_INSTALLER);
    s.owned(&executable);
    let build_script = s.root.join("build.ps1");
    s.write(&build_script, "Add-Type -Path $env:ENVSWITCH_SIGNAL_SOURCE -OutputAssembly $env:ENVSWITCH_SIGNAL_EXE -OutputType ConsoleApplication\n");
    let build_temp = s.root.join("build-temp");
    s.owned(&build_temp);
    fs::create_dir(&build_temp).unwrap();
    let built = s
        .command(resolve_tool("powershell.exe"))
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&build_script)
        .env("ENVSWITCH_SIGNAL_SOURCE", &source)
        .env("ENVSWITCH_SIGNAL_EXE", &executable)
        .env("TEMP", &build_temp)
        .env("TMP", &build_temp)
        .env("TMPDIR", &build_temp)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    s.link("powershell.exe", &executable);

    let app = s.root.join("app");
    let current = app.join("current");
    let bin = current.join("bin");
    s.owned(&app);
    fs::create_dir(&app).unwrap();
    s.owned(&current);
    fs::create_dir(&current).unwrap();
    s.owned(&bin);
    fs::create_dir(&bin).unwrap();
    let payload = bin.join(format!(
        "envswitch_{}_{}.exe",
        env!("CARGO_PKG_VERSION"),
        env!("TARGET")
    ));
    s.owned(&payload);
    fs::copy(env!("CARGO_BIN_EXE_envswitch"), &payload).unwrap();
    let install_dir = s.root.join("launchers");
    s.owned(&install_dir);
    fs::create_dir(&install_dir).unwrap();
    let receipt = json!({"schemaVersion":1,"kind":"native","repo":"demo/suite", "appRoot":app,
        "installDir":install_dir,"launcherPath":install_dir.join("envswitch.cmd"),
        "target":env!("TARGET"),"tag":format!("v{}",env!("CARGO_PKG_VERSION")),"payloadPath":payload});
    s.write(&current.join("install.json"), receipt.to_string());
    for name in ["ready", "handled", "release"] {
        s.owned(&s.root.join(name));
    }
    let mut command = s.command(&payload);
    command
        .arg("update")
        .env("ENVSWITCH_SIGNAL_ROOT", &s.root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Guard::spawn(&mut command);
    until(|| s.root.join("ready").is_file());
    send_break(child.0.as_ref().unwrap().id());
    until(|| s.root.join("handled").is_file());
    // ctrlc dispatches on another thread; keep the installer waiting while it runs.
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        child.0.as_mut().unwrap().try_wait().unwrap().is_none(),
        "parent exited before installer cleanup"
    );
    s.write(&s.root.join("release"), b"release");
    until(|| child.0.as_mut().unwrap().try_wait().unwrap().is_some());
    let output = child.0.take().unwrap().wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(7), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        "installer cleanup complete\n"
    );
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        format!(
            "envswitch [info] updating envswitch v{} -> latest\nenvswitch: update failed: installer exited with code 7\n",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert_eq!(fs::read_dir(s.root.join("TEMP")).unwrap().count(), 0);
}
