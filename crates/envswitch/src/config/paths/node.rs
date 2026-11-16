//! Port of the parts of Node's `path` module that the TS oracle calls, as
//! Bun 1.4.0 implements them: `isAbsolute`, `normalize`, `resolve`,
//! `relative`, `dirname`, `basename` and `join`, for both the POSIX and the
//! win32 flavor.
//!
//! Probed facts about Bun 1.4.0 that differ from current Node:
//!
//! * `win32.normalize` has no `.\` guard for a colon in a relative result
//!   (`normalize("a/../C:x")` is `"C:x"`, not `".\\C:x"`).
//! * `win32.relative` and `win32.resolve` fold case with ASCII rules only
//!   (`relative("C:\\É", "C:\\é\\x")` is `"..\\..\\é\\x"`).
//!
//! Everything works on UTF-8 bytes. Every character the algorithms inspect
//! or insert is ASCII and every slice boundary sits next to an ASCII byte,
//! so a UTF-8 input never gets split inside a code point, and the results
//! equal the UTF-16 results of the JS code. Both flavors compile on every
//! platform so the win32 rules can be unit tested on Unix hosts.

use crate::error::Result;

/// Lazily supplies the process cwd (`process.cwd()`), only consulted when
/// a path cannot be resolved from the arguments alone.
pub(crate) type CwdFn<'a> = &'a dyn Fn() -> Result<Vec<u8>>;

/// Lazily supplies the per-drive cwd that Windows keeps in the hidden
/// `=C:` environment variables (`process.env["=C:"]`).
pub(crate) type DriveCwdFn<'a> = &'a dyn Fn(&[u8]) -> Option<Vec<u8>>;

/// Node's `normalizeString(path, allowAboveRoot, separator, isPathSeparator)`,
/// a verbatim port (including its quirks).
fn normalize_string(
    path: &[u8],
    allow_above_root: bool,
    separator: u8,
    is_sep: fn(u8) -> bool,
) -> Vec<u8> {
    let mut res: Vec<u8> = Vec::new();
    let mut last_segment_length: isize = 0;
    let mut last_slash: isize = -1;
    let mut dots: i32 = 0;
    let mut code: u8 = 0;
    let len = path.len();
    let mut i = 0usize;
    while i <= len {
        if i < len {
            code = path[i];
        } else if is_sep(code) {
            break;
        } else {
            code = b'/';
        }

        if is_sep(code) {
            if last_slash == i as isize - 1 || dots == 1 {
                // NOOP
            } else if dots == 2 {
                let ends_with_dots =
                    res.len() >= 2 && res[res.len() - 1] == b'.' && res[res.len() - 2] == b'.';
                if res.len() < 2 || last_segment_length != 2 || !ends_with_dots {
                    if res.len() > 2 {
                        match res.iter().rposition(|&c| c == separator) {
                            None => {
                                res.clear();
                                last_segment_length = 0;
                            }
                            Some(index) => {
                                res.truncate(index);
                                let last = res.iter().rposition(|&c| c == separator);
                                last_segment_length =
                                    res.len() as isize - 1 - last.map_or(-1, |p| p as isize);
                            }
                        }
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    } else if !res.is_empty() {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    }
                }
                if allow_above_root {
                    if !res.is_empty() {
                        res.push(separator);
                    }
                    res.extend_from_slice(b"..");
                    last_segment_length = 2;
                }
            } else {
                let segment = &path[(last_slash + 1) as usize..i];
                if !res.is_empty() {
                    res.push(separator);
                }
                res.extend_from_slice(segment);
                last_segment_length = i as isize - last_slash - 1;
            }
            last_slash = i as isize;
            dots = 0;
        } else if code == b'.' && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
        i += 1;
    }
    res
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// `path.posix`.
pub(crate) mod posix {
    use super::{CwdFn, concat, normalize_string};
    use crate::error::Result;

    const SEP: u8 = b'/';

    fn is_sep(code: u8) -> bool {
        code == SEP
    }

    /// `posix.isAbsolute(path)`.
    pub(crate) fn is_absolute(path: &[u8]) -> bool {
        path.first() == Some(&SEP)
    }

    /// `posix.normalize(path)`.
    pub(crate) fn normalize(path: &[u8]) -> Vec<u8> {
        if path.is_empty() {
            return b".".to_vec();
        }
        let absolute = path[0] == SEP;
        let trailing_separator = path[path.len() - 1] == SEP;
        let mut normalized = normalize_string(path, !absolute, SEP, is_sep);
        if normalized.is_empty() {
            if absolute {
                return b"/".to_vec();
            }
            return if trailing_separator {
                b"./".to_vec()
            } else {
                b".".to_vec()
            };
        }
        if trailing_separator {
            normalized.push(SEP);
        }
        if absolute {
            concat(&[b"/", &normalized])
        } else {
            normalized
        }
    }

    /// `posix.resolve(...args)`; `cwd` stands in for `process.cwd()`.
    pub(crate) fn resolve(args: &[&[u8]], cwd: CwdFn<'_>) -> Result<Vec<u8>> {
        let mut resolved: Vec<u8> = Vec::new();
        let mut resolved_absolute = false;
        let mut index = args.len();
        let mut cwd_used = false;
        while !resolved_absolute {
            let owned;
            let path: &[u8] = if index > 0 {
                index -= 1;
                args[index]
            } else if !cwd_used {
                cwd_used = true;
                owned = cwd()?;
                &owned
            } else {
                break;
            };
            if path.is_empty() {
                continue;
            }
            resolved = concat(&[path, b"/", &resolved]);
            resolved_absolute = path[0] == SEP;
        }
        let normalized = normalize_string(&resolved, !resolved_absolute, SEP, is_sep);
        if resolved_absolute {
            return Ok(concat(&[b"/", &normalized]));
        }
        Ok(if normalized.is_empty() {
            b".".to_vec()
        } else {
            normalized
        })
    }

    /// `posix.relative(from, to)`.
    pub(crate) fn relative(from: &[u8], to: &[u8], cwd: CwdFn<'_>) -> Result<Vec<u8>> {
        if from == to {
            return Ok(Vec::new());
        }
        let from = resolve(&[from], cwd)?;
        let to = resolve(&[to], cwd)?;
        if from == to {
            return Ok(Vec::new());
        }
        let from_start = 1usize;
        let from_end = from.len();
        let from_len = from_end - from_start;
        let to_start = 1usize;
        let to_len = to.len() - to_start;

        let length = from_len.min(to_len);
        let mut last_common_sep: isize = -1;
        let mut i = 0usize;
        while i < length {
            let from_code = from[from_start + i];
            if from_code != to[to_start + i] {
                break;
            } else if from_code == SEP {
                last_common_sep = i as isize;
            }
            i += 1;
        }
        if i == length {
            if to_len > length {
                if to[to_start + i] == SEP {
                    return Ok(to[to_start + i + 1..].to_vec());
                }
                if i == 0 {
                    return Ok(to[to_start + i..].to_vec());
                }
            } else if from_len > length {
                if from[from_start + i] == SEP {
                    last_common_sep = i as isize;
                } else if i == 0 {
                    last_common_sep = 0;
                }
            }
        }

        let mut out: Vec<u8> = Vec::new();
        let mut j = (from_start as isize + last_common_sep + 1) as usize;
        while j <= from_end {
            if j == from_end || from[j] == SEP {
                out.extend_from_slice(if out.is_empty() { b".." } else { b"/.." });
            }
            j += 1;
        }
        out.extend_from_slice(&to[(to_start as isize + last_common_sep) as usize..]);
        Ok(out)
    }

    /// `posix.dirname(path)`.
    pub(crate) fn dirname(path: &[u8]) -> Vec<u8> {
        if path.is_empty() {
            return b".".to_vec();
        }
        let has_root = path[0] == SEP;
        let mut end: Option<usize> = None;
        let mut matched_slash = true;
        for i in (1..path.len()).rev() {
            if path[i] == SEP {
                if !matched_slash {
                    end = Some(i);
                    break;
                }
            } else {
                matched_slash = false;
            }
        }
        match end {
            None => {
                if has_root {
                    b"/".to_vec()
                } else {
                    b".".to_vec()
                }
            }
            Some(1) if has_root => b"//".to_vec(),
            Some(end) => path[..end].to_vec(),
        }
    }

    /// `posix.basename(path)` (no extension argument).
    pub(crate) fn basename(path: &[u8]) -> Vec<u8> {
        let mut start = 0usize;
        let mut end: Option<usize> = None;
        let mut matched_slash = true;
        for i in (0..path.len()).rev() {
            if path[i] == SEP {
                if !matched_slash {
                    start = i + 1;
                    break;
                }
            } else if end.is_none() {
                matched_slash = false;
                end = Some(i + 1);
            }
        }
        end.map_or_else(Vec::new, |end| path[start..end].to_vec())
    }

    /// `posix.join(...args)`.
    pub(crate) fn join(args: &[&[u8]]) -> Vec<u8> {
        let mut joined: Option<Vec<u8>> = None;
        for arg in args.iter().filter(|arg| !arg.is_empty()) {
            match joined.as_mut() {
                None => joined = Some(arg.to_vec()),
                Some(joined) => {
                    joined.push(SEP);
                    joined.extend_from_slice(arg);
                }
            }
        }
        joined.map_or_else(|| b".".to_vec(), |joined| normalize(&joined))
    }
}

/// `path.win32`.
pub(crate) mod win32 {
    use super::{CwdFn, DriveCwdFn, concat, normalize_string};
    use crate::error::Result;

    const BACKSLASH: u8 = b'\\';

    fn is_sep(code: u8) -> bool {
        code == b'/' || code == BACKSLASH
    }

    fn is_device_root(code: u8) -> bool {
        code.is_ascii_alphabetic()
    }

    fn at(path: &[u8], index: usize) -> u8 {
        // JS `charCodeAt` past the end is NaN, which equals no character.
        path.get(index).copied().unwrap_or(0)
    }

    /// Matches a UNC root `\\server\share` starting at index 0 (the first
    /// two bytes are known separators). Returns `(first_part_end,
    /// share_start, share_end)` when a server and share were found.
    fn match_unc(path: &[u8]) -> Option<(usize, usize, usize)> {
        let len = path.len();
        let mut j = 2;
        let mut last = j;
        while j < len && !is_sep(path[j]) {
            j += 1;
        }
        if j < len && j != last {
            let first_part_end = j;
            last = j;
            while j < len && is_sep(path[j]) {
                j += 1;
            }
            if j < len && j != last {
                last = j;
                while j < len && !is_sep(path[j]) {
                    j += 1;
                }
                return Some((first_part_end, last, j));
            }
        }
        None
    }

    /// `win32.isAbsolute(path)`.
    pub(crate) fn is_absolute(path: &[u8]) -> bool {
        let len = path.len();
        if len == 0 {
            return false;
        }
        let code = path[0];
        is_sep(code) || (len > 2 && is_device_root(code) && path[1] == b':' && is_sep(path[2]))
    }

    /// `win32.normalize(path)`.
    pub(crate) fn normalize(path: &[u8]) -> Vec<u8> {
        let len = path.len();
        if len == 0 {
            return b".".to_vec();
        }
        let mut root_end = 0usize;
        let mut device: Option<Vec<u8>> = None;
        let mut absolute = false;
        let code = path[0];

        if len == 1 {
            return if code == b'/' {
                b"\\".to_vec()
            } else {
                path.to_vec()
            };
        }
        if is_sep(code) {
            absolute = true;
            if is_sep(path[1]) {
                if let Some((first_end, share_start, share_end)) = match_unc(path) {
                    if share_end == len {
                        return concat(&[
                            b"\\\\",
                            &path[2..first_end],
                            b"\\",
                            &path[share_start..],
                            b"\\",
                        ]);
                    }
                    if share_end != share_start {
                        device = Some(concat(&[
                            b"\\\\",
                            &path[2..first_end],
                            b"\\",
                            &path[share_start..share_end],
                        ]));
                        root_end = share_end;
                    }
                }
            } else {
                root_end = 1;
            }
        } else if is_device_root(code) && path[1] == b':' {
            device = Some(path[..2].to_vec());
            root_end = 2;
            if len > 2 && is_sep(path[2]) {
                absolute = true;
                root_end = 3;
            }
        }

        let mut tail = if root_end < len {
            normalize_string(&path[root_end..], !absolute, BACKSLASH, is_sep)
        } else {
            Vec::new()
        };
        if tail.is_empty() && !absolute {
            tail = b".".to_vec();
        }
        if !tail.is_empty() && is_sep(path[len - 1]) {
            tail.push(BACKSLASH);
        }
        match device {
            None => {
                if absolute {
                    concat(&[b"\\", &tail])
                } else {
                    tail
                }
            }
            Some(device) => {
                if absolute {
                    concat(&[&device, b"\\", &tail])
                } else {
                    concat(&[&device, &tail])
                }
            }
        }
    }

    /// `win32.resolve(...args)`; `cwd` stands in for `process.cwd()` and
    /// `drive_cwd` for `process.env["=X:"]`.
    pub(crate) fn resolve(
        args: &[&[u8]],
        cwd: CwdFn<'_>,
        drive_cwd: DriveCwdFn<'_>,
    ) -> Result<Vec<u8>> {
        let mut resolved_device: Vec<u8> = Vec::new();
        let mut resolved_tail: Vec<u8> = Vec::new();
        let mut resolved_absolute = false;

        // i runs from args.len() - 1 down to -1 (the cwd step).
        let mut remaining = args.len() + 1;
        while remaining > 0 {
            remaining -= 1;
            let owned: Vec<u8>;
            let path: &[u8] = if remaining > 0 {
                let path = args[remaining - 1];
                if path.is_empty() {
                    continue;
                }
                path
            } else if resolved_device.is_empty() {
                owned = cwd()?;
                &owned
            } else {
                let mut candidate = match drive_cwd(&resolved_device) {
                    Some(value) if !value.is_empty() => value,
                    _ => cwd()?,
                };
                // `path.slice(0, 2).toLowerCase() !== resolvedDevice.toLowerCase()`
                let prefix = &candidate[..candidate.len().min(2)];
                if !prefix.eq_ignore_ascii_case(&resolved_device) && at(&candidate, 2) == BACKSLASH
                {
                    candidate = concat(&[&resolved_device, b"\\"]);
                }
                owned = candidate;
                &owned
            };

            let len = path.len();
            let mut root_end = 0usize;
            let mut device: Vec<u8> = Vec::new();
            let mut absolute = false;
            let code = at(path, 0);

            if len == 1 {
                if is_sep(code) {
                    root_end = 1;
                    absolute = true;
                }
            } else if is_sep(code) {
                absolute = true;
                if is_sep(path[1]) {
                    if let Some((first_end, share_start, share_end)) = match_unc(path)
                        && (share_end == len || share_end != share_start)
                    {
                        device = concat(&[
                            b"\\\\",
                            &path[2..first_end],
                            b"\\",
                            &path[share_start..share_end],
                        ]);
                        root_end = share_end;
                    }
                } else {
                    root_end = 1;
                }
            } else if is_device_root(code) && at(path, 1) == b':' {
                device = path[..2].to_vec();
                root_end = 2;
                if len > 2 && is_sep(path[2]) {
                    absolute = true;
                    root_end = 3;
                }
            }

            if !device.is_empty() {
                if !resolved_device.is_empty() {
                    if !device.eq_ignore_ascii_case(&resolved_device) {
                        continue;
                    }
                } else {
                    resolved_device = device;
                }
            }

            if resolved_absolute {
                if !resolved_device.is_empty() {
                    break;
                }
            } else {
                resolved_tail = concat(&[&path[root_end..], b"\\", &resolved_tail]);
                resolved_absolute = absolute;
                if absolute && !resolved_device.is_empty() {
                    break;
                }
            }
        }

        let tail = normalize_string(&resolved_tail, !resolved_absolute, BACKSLASH, is_sep);
        if resolved_absolute {
            return Ok(concat(&[&resolved_device, b"\\", &tail]));
        }
        let joined = concat(&[&resolved_device, &tail]);
        Ok(if joined.is_empty() {
            b".".to_vec()
        } else {
            joined
        })
    }

    /// `win32.relative(from, to)` with ASCII-only case folding (Bun).
    pub(crate) fn relative(
        from: &[u8],
        to: &[u8],
        cwd: CwdFn<'_>,
        drive_cwd: DriveCwdFn<'_>,
    ) -> Result<Vec<u8>> {
        if from == to {
            return Ok(Vec::new());
        }
        let from_orig = resolve(&[from], cwd, drive_cwd)?;
        let to_orig = resolve(&[to], cwd, drive_cwd)?;
        if from_orig == to_orig {
            return Ok(Vec::new());
        }
        let from = from_orig.to_ascii_lowercase();
        let to = to_orig.to_ascii_lowercase();
        if from == to {
            return Ok(Vec::new());
        }

        let mut from_start = 0usize;
        while from_start < from.len() && from[from_start] == BACKSLASH {
            from_start += 1;
        }
        let mut from_end = from.len();
        while from_end > 0 && from_end - 1 > from_start && from[from_end - 1] == BACKSLASH {
            from_end -= 1;
        }
        let from_len = from_end - from_start;

        let mut to_start = 0usize;
        while to_start < to.len() && to[to_start] == BACKSLASH {
            to_start += 1;
        }
        let mut to_end = to.len();
        while to_end > 0 && to_end - 1 > to_start && to[to_end - 1] == BACKSLASH {
            to_end -= 1;
        }
        let to_len = to_end - to_start;

        let length = from_len.min(to_len);
        let mut last_common_sep: isize = -1;
        let mut i = 0usize;
        while i < length {
            let from_code = from[from_start + i];
            if from_code != to[to_start + i] {
                break;
            } else if from_code == BACKSLASH {
                last_common_sep = i as isize;
            }
            i += 1;
        }

        if i != length {
            if last_common_sep == -1 {
                return Ok(to_orig);
            }
        } else {
            if to_len > length {
                if at(&to, to_start + i) == BACKSLASH {
                    return Ok(to_orig[to_start + i + 1..].to_vec());
                }
                if i == 2 {
                    return Ok(to_orig[to_start + i..].to_vec());
                }
            }
            if from_len > length {
                if at(&from, from_start + i) == BACKSLASH {
                    last_common_sep = i as isize;
                } else if i == 2 {
                    last_common_sep = 3;
                }
            }
            if last_common_sep == -1 {
                last_common_sep = 0;
            }
        }

        let mut out: Vec<u8> = Vec::new();
        let mut j = (from_start as isize + last_common_sep + 1) as usize;
        while j <= from_end {
            if j == from_end || from[j] == BACKSLASH {
                out.extend_from_slice(if out.is_empty() { b".." } else { b"\\.." });
            }
            j += 1;
        }

        let mut to_start = (to_start as isize + last_common_sep) as usize;
        if !out.is_empty() {
            out.extend_from_slice(slice(&to_orig, to_start, to_end));
            return Ok(out);
        }
        if at(&to_orig, to_start) == BACKSLASH {
            to_start += 1;
        }
        Ok(slice(&to_orig, to_start, to_end).to_vec())
    }

    /// JS `String.prototype.slice(start, end)` for in-range, possibly
    /// inverted indices (an inverted range is empty).
    fn slice(path: &[u8], start: usize, end: usize) -> &[u8] {
        let end = end.min(path.len());
        if start >= end { &[] } else { &path[start..end] }
    }

    /// `win32.dirname(path)`.
    pub(crate) fn dirname(path: &[u8]) -> Vec<u8> {
        let len = path.len();
        if len == 0 {
            return b".".to_vec();
        }
        let mut root_end: Option<usize> = None;
        let mut offset = 0usize;
        let code = path[0];

        if len == 1 {
            return if is_sep(code) {
                path.to_vec()
            } else {
                b".".to_vec()
            };
        }

        if is_sep(code) {
            root_end = Some(1);
            offset = 1;
            if is_sep(path[1])
                && let Some((_, share_start, share_end)) = match_unc(path)
            {
                if share_end == len {
                    return path.to_vec();
                }
                if share_end != share_start {
                    root_end = Some(share_end + 1);
                    offset = share_end + 1;
                }
            }
        } else if is_device_root(code) && path[1] == b':' {
            let end = if len > 2 && is_sep(path[2]) { 3 } else { 2 };
            root_end = Some(end);
            offset = end;
        }

        let mut end: Option<usize> = None;
        let mut matched_slash = true;
        let mut i = len;
        while i > offset {
            i -= 1;
            if is_sep(path[i]) {
                if !matched_slash {
                    end = Some(i);
                    break;
                }
            } else {
                matched_slash = false;
            }
        }

        match end.or(root_end) {
            None => b".".to_vec(),
            Some(end) => slice(path, 0, end).to_vec(),
        }
    }

    /// `win32.basename(path)` (no extension argument).
    pub(crate) fn basename(path: &[u8]) -> Vec<u8> {
        let mut start = 0usize;
        let mut end: Option<usize> = None;
        let mut matched_slash = true;
        if path.len() >= 2 && is_device_root(path[0]) && path[1] == b':' {
            start = 2;
        }
        let floor = start;
        for i in (floor..path.len()).rev() {
            if is_sep(path[i]) {
                if !matched_slash {
                    start = i + 1;
                    break;
                }
            } else if end.is_none() {
                matched_slash = false;
                end = Some(i + 1);
            }
        }
        end.map_or_else(Vec::new, |end| path[start..end].to_vec())
    }

    /// `win32.join(...args)`.
    pub(crate) fn join(args: &[&[u8]]) -> Vec<u8> {
        let mut joined: Option<Vec<u8>> = None;
        let mut first_part: &[u8] = &[];
        for arg in args.iter().filter(|arg| !arg.is_empty()) {
            match joined.as_mut() {
                None => {
                    joined = Some(arg.to_vec());
                    first_part = arg;
                }
                Some(joined) => {
                    joined.push(BACKSLASH);
                    joined.extend_from_slice(arg);
                }
            }
        }
        let Some(mut joined) = joined else {
            return b".".to_vec();
        };

        let mut needs_replace = true;
        let mut slash_count = 0usize;
        if is_sep(first_part[0]) {
            slash_count += 1;
            let first_len = first_part.len();
            if first_len > 1 && is_sep(first_part[1]) {
                slash_count += 1;
                if first_len > 2 {
                    if is_sep(first_part[2]) {
                        slash_count += 1;
                    } else {
                        needs_replace = false;
                    }
                }
            }
        }
        if needs_replace {
            while slash_count < joined.len() && is_sep(joined[slash_count]) {
                slash_count += 1;
            }
            if slash_count >= 2 {
                joined = concat(&[b"\\", &joined[slash_count..]]);
            }
        }
        normalize(&joined)
    }
}
