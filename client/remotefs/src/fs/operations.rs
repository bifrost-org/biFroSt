/* 
use std::ffi::c_int;

use fuser::Filesystem;

pub struct FilesystemImpl {
    // 1. MAPPATURE - Per tenere traccia dei file
    inode_to_path: HashMap<u64, String>,  // inode 123 -> "/remote/file.txt"
    path_to_inode: HashMap<String, u64>,  // "/remote/file.txt" -> inode 123
    
    // 2. CONTATORE - Per creare inode univoci
    next_inode: u64,  // Prossimo inode da assegnare
    
    // 3. CLIENT - Per parlare con il server (implementerai dopo)
    // client: RemoteClient,
}

impl Filesystem for FilesystemImpl {
   fn lookup(&mut self, _req: &fuser::Request<'_>, parent: u64, name: &std::ffi::OsStr, reply: fuser::ReplyEntry) {
       
   }
}
    */