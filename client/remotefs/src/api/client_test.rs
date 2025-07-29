use crate::config::settings::Config;
use super::client::{RemoteClient, ClientError};
use super::models::{WriteRequest, FileContent};
use std::time::Duration;

#[cfg(test)]
mod tests {
    use super::*;
    
    // Helper per creare un client di test configurato
    fn create_test_client() -> RemoteClient {
        let config = Config {
        server_url: "http://192.168.56.1".to_string(),
            port: 3000,
            mount_point: "/mnt/remotefs".into(),
            timeout: Duration::from_secs(60),
            username: Some("testuser".to_string()),
            password: Some("testpassword".to_string()),
            api_key: None,
        };
        
        RemoteClient::new(&config)
    }

    //problema quando voglio i file della root
    #[tokio::test]
    async fn test_list_directory() {
        let client = create_test_client();
        println!("📂 TEST: Elenco directory root");
        
        match client.list_directory("ciao").await {
            Ok(listing) => {
                println!("✅ SUCCESS: Trovati {} elementi:", listing.files.len());
                for file in &listing.files {
                    println!("  - {} ({})", file.name, 
                        if file.is_directory { "directory" } else { "file" });
                }
                assert!(true);
            }
            Err(e) => {
                println!("❌ ERROR: {}", e);
                assert!(false, "Errore nella list_directory: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_create_directory() {
        let client = create_test_client();
        let test_dir = "test_directory";
        println!("📂 TEST: Creazione directory {}", test_dir);
        
        match client.create_directory(test_dir).await {
            Ok(()) => {
                println!("✅ SUCCESS: Directory creata");
                assert!(true);
            }
            Err(e) => {
                println!("❌ ERROR: {}", e);
                assert!(false, "Errore nella create_directory: {}", e);
            }
        }
        
        // Pulizia: elimina la directory creata
        let _ = client.delete(test_dir).await;
    }

    #[tokio::test]
async fn test_url_encoding() {
    let client = create_test_client();
    
    // Test costruzione URL per path con slash
    let test_path = "/test_directory/test.txt";
    let url = format!("http://192.168.56.1:3000/files{}", test_path);
    
    println!("🔧 URL diretto: {}", url);
    
    // Test con codifica manuale
    let encoded_path = urlencoding::encode(&test_path.trim_start_matches('/'));
    let encoded_url = format!("http://192.168.56.1:3000/files/{}", encoded_path);
    
    println!("🔧 URL codificato: {}", encoded_url);
    
    // Test richiesta con path codificato
    let response = reqwest::Client::new()
        .put(&encoded_url)
        .body("test")
        .send()
        .await
        .expect("Failed to send request");
    
    println!("📋 Response status: {}", response.status());
    let body = response.text().await.expect("Failed to get body");
    println!("📋 Response body: {}", body);
}

    #[tokio::test]
    async fn test_write_and_read_file() {
        let client = create_test_client();
        let test_dir = "test_directory_for_file";
        let test_file = format!("{}/test.txt", test_dir);
        let content = "Questo è un file di test creato da RemoteClient".as_bytes().to_vec();
        
        // Prepara la directory (ignora errori se già esiste)
        let _ = client.create_directory(test_dir).await;
        
        // Test scrittura
        println!("📄 TEST: Scrittura file {}", test_file);
        let write_request = WriteRequest {
            path: test_file.clone(),
            size: Some(content.len() as u64),
            data: Some(content.clone()),
            new_path: None,
            permissions_octal: Some("rw-r--r--".to_string()),
            last_modified: Some(chrono::Utc::now().to_rfc3339()),
        };
        
        match client.write_file(&write_request).await {
            Ok(()) => println!("✅ SUCCESS: File scritto"),
            Err(e) => {
                println!("❌ ERROR: {}", e);
                assert!(false, "Errore nella write_file: {}", e);
            }
        }
        
        // Test lettura
        println!("📄 TEST: Lettura file {}", test_file);
        match client.read_file(&test_file).await {
            Ok(FileContent { data }) => {
                let content_str = String::from_utf8_lossy(&data);
                println!("✅ SUCCESS: Contenuto del file:");
                println!("---\n{}\n---", content_str);
                
                // Verifica che il contenuto letto corrisponda a quello scritto
                assert_eq!(data, content, "Il contenuto letto non corrisponde a quello scritto");
            }
            Err(e) => {
                println!("❌ ERROR: {}", e);
                assert!(false, "Errore nella read_file: {}", e);
            }
        }
        
        // Pulizia
        let _ = client.delete(&test_file).await;
        let _ = client.delete(test_dir).await;
    }

    #[tokio::test]
    async fn test_delete() {
        let client = create_test_client();
        let test_dir = "/test_directory_for_delete";
        let test_file = format!("{}/test.txt", test_dir);
        
        // Setup
        client.create_directory(test_dir).await;
        let write_request = WriteRequest {
            path: test_file.clone(),
            size: Some(4),
            data: Some(vec![1, 2, 3, 4]),
            new_path: None,
            permissions_octal: None,
            last_modified: None,
        };
        client.write_file(&write_request).await;
        
        // Test eliminazione file
        println!("🗑️ TEST: Eliminazione file {}", test_file);
        match client.delete(&test_file).await {
            Ok(()) => {
                println!("✅ SUCCESS: File eliminato");
                
                // Verifica che il file non esista più
                let check = client.get_file_metadata(&test_file).await;
                assert!(check.is_err(), "Il file esiste ancora dopo l'eliminazione");
            }
            Err(e) => {
                println!("❌ ERROR: {}", e);
                assert!(false, "Errore nella delete file: {}", e);
            }
        }
        
        // Pulizia finale
        let _ = client.delete(test_dir).await;
    }

    #[tokio::test]
async fn test_move_rename_file() {
    let client = create_test_client();
    
    // Percorsi di test
    let original_path = "/test_move_source.txt";
    let new_path = "/test_move_destination.txt";
    let content = "Contenuto file da spostare".as_bytes().to_vec();
    
    println!("📁 TEST: Move/Rename file da {} a {}", original_path, new_path);
    
    // 1. Crea il file originale
    println!("📄 STEP 1: Creazione file originale {}", original_path);
    let create_request = WriteRequest {
        path: original_path.to_string(),
        size: Some(content.len() as u64),
        data: Some(content.clone()),
        new_path: None, // Nessun move, solo creazione
        permissions_octal: Some("644".to_string()),
        last_modified: Some(chrono::Utc::now().to_rfc3339()),
    };
    
    match client.write_file(&create_request).await {
        Ok(()) => println!("✅ SUCCESS: File originale creato"),
        Err(e) => {
            println!("❌ ERROR: Errore creazione file originale: {}", e);
            assert!(false, "Fallimento creazione file: {}", e);
        }
    }
    
    // 2. Verifica che il file originale esista
    println!("🔍 STEP 2: Verifica esistenza file originale");
    match client.read_file(original_path).await {
        Ok(file_content) => {
            println!("✅ VERIFICA: File originale esiste e ha {} bytes", file_content.data.len());
            assert_eq!(file_content.data, content, "Contenuto file originale non corrisponde");
        }
        Err(e) => {
            println!("❌ ERROR: File originale non trovato: {}", e);
            assert!(false, "File originale non esiste: {}", e);
        }
    }
    
    // 3. Sposta/rinomina il file usando newPath
    println!("🔄 STEP 3: Spostamento file a {}", new_path);
    let move_request = WriteRequest {
        path: original_path.to_string(), // Path corrente (da cui spostare)
        size: Some(content.len() as u64),
        data: Some(content.clone()), // Stessi dati
        new_path: Some(new_path.to_string()), // Nuovo path (destinazione)
        permissions_octal: Some("644".to_string()),
        last_modified: Some(chrono::Utc::now().to_rfc3339()),
    };
    
    match client.write_file(&move_request).await {
        Ok(()) => println!("✅ SUCCESS: File spostato con successo"),
        Err(e) => {
            println!("❌ ERROR: Errore spostamento file: {}", e);
            assert!(false, "Fallimento spostamento file: {}", e);
        }
    }
    
    // 4. Verifica che il file sia nel nuovo percorso
    println!("🔍 STEP 4: Verifica file nel nuovo percorso");
    match client.read_file(new_path).await {
        Ok(file_content) => {
            println!("✅ VERIFICA: File trovato nel nuovo percorso con {} bytes", file_content.data.len());
            assert_eq!(file_content.data, content, "Contenuto file spostato non corrisponde");
        }
        Err(e) => {
            println!("❌ ERROR: File non trovato nel nuovo percorso: {}", e);
            assert!(false, "File non spostato correttamente: {}", e);
        }
    }
    
    // 5. Verifica che il file originale sia stato rimosso
    println!("🔍 STEP 5: Verifica rimozione file originale");
    match client.read_file(original_path).await {
        Ok(_) => {
            println!("❌ VERIFICA FALLITA: File originale ancora presente");
            assert!(false, "File originale non è stato rimosso dopo lo spostamento");
        }
        Err(ClientError::NotFound { .. }) => {
            println!("✅ VERIFICA: File originale correttamente rimosso");
        }
        Err(e) => {
            println!("❌ ERROR: Errore imprevisto durante verifica rimozione: {}", e);
        }
    }
    
    // 6. Pulizia: elimina il file spostato
    println!("🧹 CLEANUP: Rimozione file di test");
    let _ = client.delete(new_path).await;
    
    println!("✅ TEST COMPLETATO: Move/Rename file funziona correttamente");
}

#[tokio::test]
async fn test_move_to_subdirectory() {
    let client = create_test_client();
    
    // Test spostamento in sottodirectory
    let original_path = "/file_to_move.txt";
    let test_dir = "/test_move_dir";
    let new_path = "/test_move_dir/moved_file.txt";
    let content = "File da spostare in subdirectory".as_bytes().to_vec();
    
    println!("📁 TEST: Move file in subdirectory da {} a {}", original_path, new_path);
    
    // 1. Crea directory di destinazione
    println!("📂 STEP 1: Creazione directory {}", test_dir);
    match client.create_directory(test_dir).await {
        Ok(()) => println!("✅ SUCCESS: Directory creata"),
        Err(e) => println!("⚠️  WARNING: Errore creazione directory (potrebbe già esistere): {}", e),
    }
    
    // 2. Crea file originale
    println!("📄 STEP 2: Creazione file originale");
    let create_request = WriteRequest {
        path: original_path.to_string(),
        size: Some(content.len() as u64),
        data: Some(content.clone()),
        new_path: None,
        permissions_octal: Some("644".to_string()),
        last_modified: Some(chrono::Utc::now().to_rfc3339()),
    };
    
    match client.write_file(&create_request).await {
        Ok(()) => println!("✅ SUCCESS: File originale creato"),
        Err(e) => {
            println!("❌ ERROR: {}", e);
            assert!(false, "Fallimento creazione file: {}", e);
        }
    }
    
    // 3. Sposta il file nella subdirectory
    println!("🔄 STEP 3: Spostamento in subdirectory");
    let move_request = WriteRequest {
        path: original_path.to_string(),
        size: Some(content.len() as u64),
        data: Some(content.clone()),
        new_path: Some(new_path.to_string()),
        permissions_octal: Some("644".to_string()),
        last_modified: Some(chrono::Utc::now().to_rfc3339()),
    };
    
    match client.write_file(&move_request).await {
        Ok(()) => println!("✅ SUCCESS: File spostato in subdirectory"),
        Err(e) => {
            println!("❌ ERROR: {}", e);
            assert!(false, "Fallimento spostamento: {}", e);
        }
    }
    
    // 4. Verifica nel nuovo percorso
    match client.read_file(new_path).await {
        Ok(file_content) => {
            println!("✅ VERIFICA: File trovato in subdirectory");
            assert_eq!(file_content.data, content);
        }
        Err(e) => {
            println!("❌ ERROR: {}", e);
            assert!(false, "File non trovato in subdirectory: {}", e);
        }
    }
    
    // Pulizia
    let _ = client.delete(new_path).await;
    let _ = client.delete(test_dir).await;
    let _ = client.delete(original_path).await; // Nel caso non sia stato spostato
    
    println!("✅ TEST COMPLETATO: Move in subdirectory funziona");
}

#[tokio::test]
async fn test_rename_in_place() {
    let client = create_test_client();
    
    // Test rinominazione senza spostamento (stessa directory)
    let original_path = "/original_name.txt";
    let new_path = "/renamed_file.txt";
    let content = "File da rinominare".as_bytes().to_vec();
    
    println!("📝 TEST: Rinominazione file da {} a {}", original_path, new_path);
    
    // 1. Crea file originale
    let create_request = WriteRequest {
        path: original_path.to_string(),
        size: Some(content.len() as u64),
        data: Some(content.clone()),
        new_path: None,
        permissions_octal: Some("644".to_string()),
        last_modified: Some(chrono::Utc::now().to_rfc3339()),
    };
    
    match client.write_file(&create_request).await {
        Ok(()) => println!("✅ SUCCESS: File originale creato"),
        Err(e) => assert!(false, "Fallimento creazione: {}", e),
    }
    
    // 2. Rinomina il file
    let rename_request = WriteRequest {
        path: original_path.to_string(),
        size: Some(content.len() as u64),
        data: Some(content.clone()),
        new_path: Some(new_path.to_string()), // Nuovo nome
        permissions_octal: Some("644".to_string()),
        last_modified: Some(chrono::Utc::now().to_rfc3339()),
    };
    
    match client.write_file(&rename_request).await {
        Ok(()) => println!("✅ SUCCESS: File rinominato"),
        Err(e) => assert!(false, "Fallimento rinominazione: {}", e),
    }
    
    // 3. Verifica nuovo nome
    match client.read_file(new_path).await {
        Ok(_) => println!("✅ VERIFICA: File rinominato trovato"),
        Err(e) => assert!(false, "File rinominato non trovato: {}", e),
    }
    
    // 4. Verifica rimozione vecchio nome
    match client.read_file(original_path).await {
        Ok(_) => assert!(false, "File con vecchio nome ancora presente"),
        Err(ClientError::NotFound { .. }) => println!("✅ VERIFICA: Vecchio nome rimosso"),
        Err(e) => println!("⚠️  WARNING: Errore verifica: {}", e),
    }
    
    // Pulizia
    let _ = client.delete(new_path).await;
    
    println!("✅ TEST COMPLETATO: Rinominazione funziona");
}

// non corrispondono i permessi
#[tokio::test]
async fn test_get_file_metadata() {
    let client = create_test_client();
    
    // Setup: crea un file di test
    let test_file = "/metadata_test_file.txt";
    let content = "File per testare i metadati".as_bytes().to_vec();
    
    println!("📋 TEST: Get file metadata per {}", test_file);
    
    // 1. Crea il file
    println!("📄 STEP 1: Creazione file di test");
    let write_request = WriteRequest {
        path: test_file.to_string(),
        size: Some(content.len() as u64),
        data: Some(content.clone()),
        new_path: None,
        permissions_octal: Some("644".to_string()),
        last_modified: Some(chrono::Utc::now().to_rfc3339()),
    };
    
    match client.write_file(&write_request).await {
        Ok(()) => println!("✅ SUCCESS: File di test creato"),
        Err(e) => {
            println!("❌ ERROR: Errore creazione file: {}", e);
            assert!(false, "Fallimento creazione file: {}", e);
        }
    }
    
    // 2. Ottieni i metadati del file
    println!("🔍 STEP 2: Recupero metadati file");
    match client.get_file_metadata(test_file).await {
        Ok(metadata) => {
            println!("✅ SUCCESS: Metadati recuperati");
            println!("  📁 Nome: {}", metadata.name);
            println!("  📏 Dimensione: {} bytes", metadata.size);
            println!("  🔒 Permessi: {}", metadata.permissions_octal);
            println!("  📅 Ultima modifica: {}", metadata.last_modified);
            println!("  📂 È directory: {}", metadata.is_directory);
            
            // Verifica che i metadati siano corretti
            assert_eq!(metadata.name, test_file, "Nome file non corrisponde");
            assert_eq!(metadata.size, content.len() as u64, "Dimensione file non corrisponde");
            assert!(!metadata.is_directory, "Il file non dovrebbe essere marcato come directory");
            assert!(metadata.permissions_octal.contains("64"), "Permessi non corrispondono");
            
            println!("✅ VERIFICA: Tutti i metadati sono corretti");
        }
        Err(e) => {
            println!("❌ ERROR: Errore recupero metadati: {}", e);
            assert!(false, "Fallimento get_file_metadata: {}", e);
        }
    }
    
    // Pulizia
    let _ = client.delete(test_file).await;
    println!("✅ TEST COMPLETATO: get_file_metadata funziona correttamente");
}

#[tokio::test]
async fn test_get_directory_metadata() {
    let client = create_test_client();
    
    let test_dir = "/metadata_test_directory";
    
    println!("📂 TEST: Get directory metadata per {}", test_dir);
    
    // 1. Crea la directory
    println!("📁 STEP 1: Creazione directory di test");
    match client.create_directory(test_dir).await {
        Ok(()) => println!("✅ SUCCESS: Directory di test creata"),
        Err(e) => {
            println!("❌ ERROR: Errore creazione directory: {}", e);
            assert!(false, "Fallimento creazione directory: {}", e);
        }
    }
    
    // 2. Ottieni i metadati della directory
    println!("🔍 STEP 2: Recupero metadati directory");
    match client.get_file_metadata(test_dir).await {
        Ok(metadata) => {
            println!("✅ SUCCESS: Metadati directory recuperati");
            println!("  📁 Nome: {}", metadata.name);
            println!("  📏 Dimensione: {} bytes", metadata.size);
            println!("  🔒 Permessi: {}", metadata.permissions_octal);
            println!("  📅 Ultima modifica: {}", metadata.last_modified);
            println!("  📂 È directory: {}", metadata.is_directory);
            
            // Verifica che sia marcata come directory
            assert!(metadata.is_directory, "La directory dovrebbe essere marcata come directory");
            assert_eq!(metadata.name, test_dir, "Nome directory non corrisponde");
            
            println!("✅ VERIFICA: Metadati directory corretti");
        }
        Err(e) => {
            println!("❌ ERROR: Errore recupero metadati directory: {}", e);
            assert!(false, "Fallimento get_file_metadata per directory: {}", e);
        }
    }
    
    // Pulizia
    let _ = client.delete(test_dir).await;
    println!("✅ TEST COMPLETATO: get_file_metadata per directory funziona");
}

#[tokio::test]
async fn test_get_file_metadata_nested() {
    let client = create_test_client();
    
    // Test con file in subdirectory
    let test_dir = "/nested_test_dir";
    let test_file = "/nested_test_dir/nested_file.txt";
    let content = "File in subdirectory".as_bytes().to_vec();
    
    println!("📂 TEST: Get metadata file in subdirectory {}", test_file);
    
    // 1. Crea directory
    println!("📁 STEP 1: Creazione directory");
    match client.create_directory(test_dir).await {
        Ok(()) => println!("✅ SUCCESS: Directory creata"),
        Err(e) => println!("⚠️  WARNING: {}", e),
    }
    
    // 2. Crea file in subdirectory
    println!("📄 STEP 2: Creazione file in subdirectory");
    let write_request = WriteRequest {
        path: test_file.to_string(),
        size: Some(content.len() as u64),
        data: Some(content.clone()),
        new_path: None,
        permissions_octal: Some("644".to_string()),
        last_modified: Some(chrono::Utc::now().to_rfc3339()),
    };
    
    match client.write_file(&write_request).await {
        Ok(()) => println!("✅ SUCCESS: File in subdirectory creato"),
        Err(e) => assert!(false, "Fallimento creazione file: {}", e),
    }
    
    // 3. Ottieni metadati del file nested
    println!("🔍 STEP 3: Recupero metadati file nested");
    match client.get_file_metadata(test_file).await {
        Ok(metadata) => {
            println!("✅ SUCCESS: Metadati file nested recuperati");
            println!("  📁 Nome: {}", metadata.name);
            println!("  📏 Dimensione: {} bytes", metadata.size);
            println!("  📂 È directory: {}", metadata.is_directory);
            
            assert_eq!(metadata.name, test_file, "Nome file nested non corrisponde");
            assert_eq!(metadata.size, content.len() as u64, "Dimensione file nested non corrisponde");
            assert!(!metadata.is_directory, "File nested non dovrebbe essere directory");
            
            println!("✅ VERIFICA: Metadati file nested corretti");
        }
        Err(e) => assert!(false, "Fallimento get_file_metadata per file nested: {}", e),
    }
    
    // Pulizia
    let _ = client.delete(test_file).await;
    let _ = client.delete(test_dir).await;
    println!("✅ TEST COMPLETATO: get_file_metadata per file nested funziona");
}

#[tokio::test]
async fn test_get_file_metadata_not_found() {
    let client = create_test_client();
    
    let non_existent_file = "/this_file_does_not_exist.txt";
    
    println!("❌ TEST: Get metadata file inesistente {}", non_existent_file);
    
    // Tenta di ottenere metadati di un file che non esiste
    match client.get_file_metadata(non_existent_file).await {
        Ok(_) => {
            println!("❌ ERROR: Il file inesistente ha restituito metadati");
            assert!(false, "File inesistente non dovrebbe restituire metadati");
        }
        Err(ClientError::NotFound { path }) => {
            println!("✅ SUCCESS: Correttamente restituito NotFound per file inesistente");
            println!("  📁 Path: {}", path);
            assert_eq!(path, non_existent_file, "Path nell'errore non corrisponde");
        }
        Err(e) => {
            println!("❌ ERROR: Errore imprevisto: {}", e);
            assert!(false, "Errore imprevisto per file inesistente: {}", e);
        }
    }
    
    println!("✅ TEST COMPLETATO: get_file_metadata gestisce correttamente file inesistenti");
}

#[tokio::test]
async fn test_get_file_metadata_root_files() {
    let client = create_test_client();
    
    println!("📂 TEST: Get metadata files nella root");
    
    // Prima ottieni la lista dei file nella root
    match client.list_directory("/").await {
        Ok(listing) => {
            if listing.files.is_empty() {
                println!("⚠️  WARNING: Nessun file nella root per testare");
                return;
            }
            
            println!("🔍 Trovati {} file nella root", listing.files.len());
            
            // Testa i metadati del primo file
            let first_file = &listing.files[0];
            println!("🔍 Testing metadati per: {}", first_file.name);
            
            match client.get_file_metadata(&first_file.name).await {
                Ok(metadata) => {
                    println!("✅ SUCCESS: Metadati recuperati per file esistente");
                    println!("  📁 Nome: {}", metadata.name);
                    println!("  📏 Dimensione: {} bytes", metadata.size);
                    println!("  📂 È directory: {}", metadata.is_directory);
                    
                    // Verifica che i metadati corrispondano a quelli del listing
                    assert_eq!(metadata.name, first_file.name);
                    assert_eq!(metadata.size, first_file.size);
                    assert_eq!(metadata.is_directory, first_file.is_directory);
                    
                    println!("✅ VERIFICA: Metadati consistenti con il listing");
                }
                Err(e) => {
                    println!("❌ ERROR: {}", e);
                    assert!(false, "Fallimento get_file_metadata per file esistente: {}", e);
                }
            }
        }
        Err(e) => {
            println!("❌ ERROR: Impossibile listare directory root: {}", e);
            assert!(false, "Fallimento list_directory: {}", e);
        }
    }
    
    println!("✅ TEST COMPLETATO: get_file_metadata per file root funziona");
}


}