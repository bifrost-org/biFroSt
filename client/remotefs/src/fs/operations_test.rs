#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::ClientError;
    use crate::api::models::MetaFile;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ffi::OsStr;

    // Mock semplice di RemoteClient
    struct MockClient {
        responses: RefCell<HashMap<String, Result<MetaFile, ClientError>>>,
    }

    impl MockClient {
        fn new() -> Self {
            Self {
                responses: RefCell::new(HashMap::new()),
            }
        }

        fn add_response(&self, path: &str, result: Result<MetaFile, ClientError>) {
            self.responses.borrow_mut().insert(path.to_string(), result);
        }

        // Versione sincrona per semplicità
        fn get_file_metadata(&self, path: &str) -> Result<MetaFile, ClientError> {
            match self.responses.borrow().get(path) {
                Some(result) => match result {
                    Ok(meta) => Ok(meta.clone()),
                    Err(e) => Err(ClientError::NotFound {
                        path: path.to_string(),
                    }),
                },
                None => Err(ClientError::NotFound {
                    path: path.to_string(),
                }),
            }
        }
    }

    // Implementazione semplificata di RemoteFileSystem per test
    struct TestFileSystem {
        inode_to_path: HashMap<u64, String>,
        path_to_inode: HashMap<String, u64>,
        next_inode: u64,
        client: MockClient,
    }

    impl TestFileSystem {
        fn new(client: MockClient) -> Self {
            let mut fs = Self {
                inode_to_path: HashMap::new(),
                path_to_inode: HashMap::new(),
                next_inode: 2,
                client,
            };
            fs.inode_to_path.insert(1, "/".to_string());
            fs.path_to_inode.insert("/".to_string(), 1);
            fs
        }

        // Metodi helper identici a RemoteFileSystem
        fn generate_inode(&mut self) -> u64 {
            let inode = self.next_inode;
            self.next_inode += 1;
            inode
        }

        fn get_path(&self, inode: u64) -> Option<&String> {
            self.inode_to_path.get(&inode)
        }

        fn register_inode(&mut self, inode: u64, path: String) {
            self.inode_to_path.insert(inode, path.clone());
            self.path_to_inode.insert(path, inode);
        }

        // Versione semplificata di lookup per test
        fn lookup(&mut self, parent: u64, name: &str) -> Result<u64, i32> {
            let parent_path = match self.get_path(parent) {
                Some(path) => path.clone(),
                None => return Err(libc::ENOENT),
            };

            let full_path = if parent_path == "/" {
                format!("/{}", name)
            } else {
                format!("{}/{}", parent_path, name)
            };

            // Check nella cache
            if let Some(&existing_inode) = self.path_to_inode.get(&full_path) {
                return Ok(existing_inode);
            }

            // Usa client mock
            match self.client.get_file_metadata(&full_path) {
                Ok(_metadata) => {
                    let new_inode = self.generate_inode();
                    self.register_inode(new_inode, full_path);
                    Ok(new_inode)
                }
                Err(ClientError::NotFound { .. }) => Err(libc::ENOENT),
                Err(_) => Err(libc::EIO),
            }
        }
    }

    #[test]
    fn test_lookup_basic() {
        // Setup
        let client = MockClient::new();
        let mut fs = TestFileSystem::new(client);

        // Test 1: File che non esiste
        let result = fs.lookup(1, "nonexistent.txt");
        assert_eq!(result, Err(libc::ENOENT));

        // Test 2: Aggiungi risposta per file che esiste
        let test_file = MetaFile {
            name: "/test.txt".to_string(),
            size: 100,
            last_modified: "2025-01-01T00:00:00Z".to_string(),
            permissions_octal: "644".to_string(),
            is_directory: false,
        };
        fs.client.add_response("/test.txt", Ok(test_file));

        // Ora il file dovrebbe essere trovato
        let inode = fs.lookup(1, "test.txt").expect("File dovrebbe esistere");
        assert!(inode > 1, "Dovrebbe essere assegnato un nuovo inode");
        assert_eq!(fs.get_path(inode), Some(&"/test.txt".to_string()));
    }
}
