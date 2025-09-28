use chrono::DateTime;
use fuser::{FileAttr, FileType};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::api::models::MetaFile;

pub fn new_file_attr(ino: u64, size: u64, permission_octal: u32) -> FileAttr {
    let now = SystemTime::now();
    FileAttr {
        ino,
        size,
        blocks: (size + 511) / 512,
        atime: now,
        mtime: now,
        ctime: now,
        crtime: now,
        kind: FileType::RegularFile,
        perm: permission_octal as u16,
        nlink: 1,
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}

pub fn new_directory_attr(ino: u64, permission_octal: u32) -> FileAttr {
    let now = SystemTime::now();
    FileAttr {
        ino,
        size: 4096,
        blocks: 8,
        atime: now,
        mtime: now,
        ctime: now,
        crtime: now,
        kind: FileType::Directory,
        perm: permission_octal as u16,
        nlink: 2,
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}
fn parse_permissions(perm: &str) -> u16 {
    if perm.len() == 3 && perm.chars().all(|c| c.is_ascii_digit() && c <= '7') {
        return u16::from_str_radix(perm, 8).unwrap_or(0o644);
    }

    if perm.len() == 9 && (perm.starts_with('r') || perm.starts_with('-')) {
        return symbolic_to_octal(perm);
    }

    if let Ok(decimal_perm) = perm.parse::<u32>() {
        if decimal_perm <= 777 && decimal_perm.to_string().chars().all(|c| c <= '7') {
            return decimal_perm as u16;
        }
        return decimal_perm as u16;
    }

    let octal_str = match perm {
        "rw-r--r--" => "644",
        "rwxr-xr-x" => "755",
        "rw-------" => "600",
        "rwxrwxrwx" => "777",
        "r--r--r--" => "444",
        "rwxrwxr-x" => "775",
        _ => "644",
    };

    u16::from_str_radix(octal_str, 8).unwrap_or(0o644)
}

fn symbolic_to_octal(symbolic: &str) -> u16 {
    let mut octal = 0u16;

    if symbolic.chars().nth(0) == Some('r') {
        octal += 0o400;
    }
    if symbolic.chars().nth(1) == Some('w') {
        octal += 0o200;
    }
    if symbolic.chars().nth(2) == Some('x') {
        octal += 0o100;
    }

    if symbolic.chars().nth(3) == Some('r') {
        octal += 0o040;
    }
    if symbolic.chars().nth(4) == Some('w') {
        octal += 0o020;
    }
    if symbolic.chars().nth(5) == Some('x') {
        octal += 0o010;
    }

    if symbolic.chars().nth(6) == Some('r') {
        octal += 0o004;
    }
    if symbolic.chars().nth(7) == Some('w') {
        octal += 0o002;
    }
    if symbolic.chars().nth(8) == Some('x') {
        octal += 0o001;
    }

    octal
}

pub fn from_metadata(new_inode: u64, metadata: &MetaFile) -> FileAttr {
    let parse_timestamp = |timestamp_str: &str| -> SystemTime {
        if let Ok(dt) = DateTime::parse_from_rfc3339(timestamp_str) {
            let unix_timestamp = dt.timestamp() as u64;
            let nanos = dt.timestamp_subsec_nanos();
            return UNIX_EPOCH + Duration::new(unix_timestamp, nanos);
        }

        if let Ok(secs) = timestamp_str.parse::<u64>() {
            return UNIX_EPOCH + Duration::from_secs(secs);
        }

        SystemTime::now()
    };

    FileAttr {
        ino: new_inode,
        size: metadata.size,
        blocks: (metadata.size + 511) / 512,
        atime: parse_timestamp(&metadata.atime),
        mtime: parse_timestamp(&metadata.mtime),
        ctime: parse_timestamp(&metadata.ctime),
        crtime: parse_timestamp(&metadata.crtime),
        kind: match metadata.kind {
            crate::api::models::FileKind::RegularFile => FileType::RegularFile,
            crate::api::models::FileKind::Directory => FileType::Directory,
            crate::api::models::FileKind::Symlink => FileType::Symlink,
            crate::api::models::FileKind::Hardlink => FileType::RegularFile,
        },
        perm: parse_permissions(&metadata.perm),
        nlink: metadata.nlink,
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}
