//! Trusted installers compiled into the binary, independent of installed scripts.

pub(crate) fn installer_script() -> (&'static str, &'static str) {
    if cfg!(windows) {
        (
            "install.ps1",
            include_str!("../../../../scripts/install.ps1"),
        )
    } else {
        ("install.sh", include_str!("../../../../scripts/install.sh"))
    }
}
