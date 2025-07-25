use fuser::{FileAttr, FileType};
use std::time::SystemTime;

use crate::api::models::MetaFile;

// Funzioni helper per creare FileAttr
pub fn new_file_attr(ino: u64, size: u64) -> FileAttr {
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
        perm: 0o644,
        nlink: 1,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}

pub fn new_directory_attr(ino: u64) -> FileAttr {
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
        perm: 0o755,
        nlink: 2,
        uid: 1000,
        gid: 1000,
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}

pub fn from_metadata(new_inode: u64, metadata: &MetaFile) -> FileAttr {
    FileAttr {
        ino: new_inode,
        size: metadata.size,
        blocks: (metadata.size + 511) / 512,
        atime: SystemTime::now(), // Non abbiamo accesso a questi dati
        mtime: SystemTime::now(),
        ctime: SystemTime::now(),
        crtime: SystemTime::now(),
        kind: if metadata.name.as_str().contains('.') {
            FileType::RegularFile
        } else {
            FileType::Directory
        },
        perm: u16::from_str_radix(&metadata.permissions_octal, 8).unwrap_or(0o644),
        nlink: 1, // Non gestiamo link multipli
        uid: 1000, // ID utente fittizio
        gid: 1000, // ID gruppo fittizio
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}