use chrono::DateTime;
use fuser::{FileAttr, FileType};
use std::time::{SystemTime, UNIX_EPOCH, Duration};

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
        uid: 1000,
        gid: 1000,
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
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}

fn parse_permissions(perm: &str) -> u16 {
    let s = perm.trim();

    // 3-digit octal like "644"
    if s.len() == 3 && s.chars().all(|c| ('0'..='7').contains(&c)) {
        return u16::from_str_radix(s, 8).unwrap_or(0o644);
    }

    // 4-digit with leading zero like "0644"
    if s.len() == 4 && s.starts_with('0') && s.chars().skip(1).all(|c| ('0'..='7').contains(&c)) {
        return u16::from_str_radix(&s[1..], 8).unwrap_or(0o644);
    }

    // symbolic form like "rw-r--r--"
    if s.len() == 9 && s.chars().all(|c| matches!(c, 'r' | 'w' | 'x' | '-')) {
        return symbolic_to_octal(s);
    }

    // pure numeric string (take last 3 digits if longer)
    if s.chars().all(|c| c.is_ascii_digit()) {
        let trimmed = if s.len() > 3 { &s[s.len() - 3..] } else { s };
        if trimmed.chars().all(|c| ('0'..='7').contains(&c)) {
            return u16::from_str_radix(trimmed, 8).unwrap_or(0o644);
        }
    }

    // fallback mapping for some common strings (keeps previous behavior)
    let octal_str = match s {
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
    let b = symbolic.as_bytes();
    if b.len() != 9 {
        return 0o644;
    }
    let mut octal = 0u16;

    if b[0] == b'r' { octal |= 0o400; }
    if b[1] == b'w' { octal |= 0o200; }
    if b[2] == b'x' { octal |= 0o100; }

    if b[3] == b'r' { octal |= 0o040; }
    if b[4] == b'w' { octal |= 0o020; }
    if b[5] == b'x' { octal |= 0o010; }

    if b[6] == b'r' { octal |= 0o004; }
    if b[7] == b'w' { octal |= 0o002; }
    if b[8] == b'x' { octal |= 0o001; }

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
        uid: 1000, 
        gid: 1000, 
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}