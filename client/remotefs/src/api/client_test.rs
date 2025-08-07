use super::client::{ClientError, RemoteClient};
use super::models::{FileContent, WriteRequest};
use crate::config::settings::Config;
use std::time::Duration;
/* 
#[cfg(test)]
mod tests {
    use crate::api::models::{FileKind, Mode};

    use super::*;

    // Helper per creare un client di test configurato
    fn create_test_client() -> RemoteClient {
        let config = Config {
            server_url: "https://bifrost.oberon-server.it".to_string(),
            port: 443,
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
        println!("📂 TEST: Elenco directory");

        // Test prima con la root
        match client.list_directory("/").await {
            Ok(listing) => {
                println!(
                    "✅ SUCCESS ROOT: Trovati {} elementi nella root:",
                    listing.files.len()
                );
                for file in &listing.files {
                    println!(
                        "  - {} ({}, {} bytes, perm: {})",
                        file.name,
                        match file.kind {
                            FileKind::Directory => "directory",
                            FileKind::RegularFile => "file",
                            FileKind::Symlink => "symlink",
                            FileKind::Hardlink => "hardlink",
                        },
                        file.size,
                        file.perm
                    );
                }
            }
            Err(e) => {
                println!("❌ ERROR ROOT: {}", e);
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
    async fn test_write_and_read_file() {
        let client = create_test_client();
        let test_dir = "test_directory_for_file";
        let test_file = format!("{}/test.txt", test_dir);
        let content = "Questo è un file di test creato da RemoteClient"
            .as_bytes()
            .to_vec();

        // Prepara la directory (ignora errori se già esiste)
        let _ = client.create_directory(test_dir).await;
        let now = chrono::Utc::now().to_rfc3339();
        // Test scrittura
        println!("📄 TEST: Scrittura file {}", test_file);
        let write_request = WriteRequest {
            path: test_file.clone(),
            size: content.len() as u64,
            data: Some(content.clone()),
            new_path: None,
            perm: "777".to_string(),
            atime: now.clone(),
            mtime: now.clone(),
            ctime: now.clone(),
            crtime: now.clone(),
            kind: FileKind::RegularFile,
            ref_path: None,
            mode: Mode::Write,
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
        match client.read_file(&test_file, None, None).await {
            Ok(FileContent { data }) => {
                let content_str = String::from_utf8_lossy(&data);
                println!("✅ SUCCESS: Contenuto del file:");
                println!("---\n{}\n---", content_str);

                // Verifica che il contenuto letto corrisponda a quello scritto
                assert_eq!(
                    data, content,
                    "Il contenuto letto non corrisponde a quello scritto"
                );
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
        println!("📄 SETUP: Creazione file {}", test_file);
        let now = chrono::Utc::now().to_rfc3339();
        let write_request = WriteRequest {
            path: test_file.clone(),
            new_path: None,
            size: 4,
            atime: now.clone(),
            mtime: now.clone(),
            ctime: now.clone(),
            crtime: now.clone(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
            data: Some(vec![1, 2, 3, 4]),
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

        println!(
            "📁 TEST: Move/Rename file da {} a {}",
            original_path, new_path
        );

        // 1. Crea il file originale
        println!("📄 STEP 1: Creazione file originale {}", original_path);
        let create_request = WriteRequest {
            path: original_path.to_string(),
            size: content.len() as u64,
            data: Some(content.clone()),
            new_path: None, // Nessun move, solo creazione
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
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
        match client.read_file(original_path, None, None).await {
            Ok(file_content) => {
                println!(
                    "✅ VERIFICA: File originale esiste e ha {} bytes",
                    file_content.data.len()
                );
                assert_eq!(
                    file_content.data, content,
                    "Contenuto file originale non corrisponde"
                );
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
            size: content.len() as u64,
            data: Some(content.clone()),          // Stessi dati
            new_path: Some(new_path.to_string()), // Nuovo path (destinazione)
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::WriteAt, // Usa WriteAt per spostare
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
        match client.read_file(new_path, None, None).await {
            Ok(file_content) => {
                println!(
                    "✅ VERIFICA: File trovato nel nuovo percorso con {} bytes",
                    file_content.data.len()
                );
                assert_eq!(
                    file_content.data, content,
                    "Contenuto file spostato non corrisponde"
                );
            }
            Err(e) => {
                println!("❌ ERROR: File non trovato nel nuovo percorso: {}", e);
                assert!(false, "File non spostato correttamente: {}", e);
            }
        }

        // 5. Verifica che il file originale sia stato rimosso
        println!("🔍 STEP 5: Verifica rimozione file originale");
        match client.read_file(original_path, None, None).await {
            Ok(_) => {
                println!("❌ VERIFICA FALLITA: File originale ancora presente");
                assert!(
                    false,
                    "File originale non è stato rimosso dopo lo spostamento"
                );
            }
            Err(ClientError::NotFound { .. }) => {
                println!("✅ VERIFICA: File originale correttamente rimosso");
            }
            Err(e) => {
                println!(
                    "❌ ERROR: Errore imprevisto durante verifica rimozione: {}",
                    e
                );
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

        println!(
            "📁 TEST: Move file in subdirectory da {} a {}",
            original_path, new_path
        );

        // 1. Crea directory di destinazione
        println!("📂 STEP 1: Creazione directory {}", test_dir);
        match client.create_directory(test_dir).await {
            Ok(()) => println!("✅ SUCCESS: Directory creata"),
            Err(e) => println!(
                "⚠️  WARNING: Errore creazione directory (potrebbe già esistere): {}",
                e
            ),
        }

        // 2. Crea file originale
        println!("📄 STEP 2: Creazione file originale");
        let create_request = WriteRequest {
            path: original_path.to_string(),
            size: content.len() as u64,
            data: Some(content.clone()),
            new_path: None,
            atime: chrono::Utc::now().to_rfc3339(),

            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
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
            size: content.len() as u64,
            data: Some(content.clone()),
            new_path: Some(new_path.to_string()),
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::WriteAt, // Usa WriteAt per spostare
        };

        match client.write_file(&move_request).await {
            Ok(()) => println!("✅ SUCCESS: File spostato in subdirectory"),
            Err(e) => {
                println!("❌ ERROR: {}", e);
                assert!(false, "Fallimento spostamento: {}", e);
            }
        }

        // 4. Verifica nel nuovo percorso
        match client.read_file(new_path, None, None).await {
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

        println!(
            "📝 TEST: Rinominazione file da {} a {}",
            original_path, new_path
        );

        // 1. Crea file originale
        let create_request = WriteRequest {
            path: original_path.to_string(),
            size: content.len() as u64,
            data: Some(content.clone()),
            new_path: None,
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
        };

        match client.write_file(&create_request).await {
            Ok(()) => println!("✅ SUCCESS: File originale creato"),
            Err(e) => assert!(false, "Fallimento creazione: {}", e),
        }

        // 2. Rinomina il file
        let rename_request = WriteRequest {
            path: original_path.to_string(),
            size: content.len() as u64,
            data: Some(content.clone()),
            new_path: Some(new_path.to_string()), // Nuovo nome
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
        };

        match client.write_file(&rename_request).await {
            Ok(()) => println!("✅ SUCCESS: File rinominato"),
            Err(e) => assert!(false, "Fallimento rinominazione: {}", e),
        }

        // 3. Verifica nuovo nome
        match client.read_file(new_path, None, None).await {
            Ok(_) => println!("✅ VERIFICA: File rinominato trovato"),
            Err(e) => assert!(false, "File rinominato non trovato: {}", e),
        }

        // 4. Verifica rimozione vecchio nome
        match client.read_file(original_path, None, None).await {
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
            size: content.len() as u64,
            data: Some(content.clone()),
            new_path: None,
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
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
                println!("  🔒 Permessi: {}", metadata.perm);
                println!("  📅 Ultima modifica: {}", metadata.mtime);
                println!("  📂 È directory: {:?}", metadata.kind);

                // Verifica che i metadati siano corretti
                assert_eq!(metadata.name, test_file, "Nome file non corrisponde");
                assert_eq!(
                    metadata.size,
                    content.len() as u64,
                    "Dimensione file non corrisponde"
                );
                assert!(
                    match metadata.kind {
                        FileKind::Directory => true,
                        _ => false,
                    },
                    "Il file non dovrebbe essere marcato come directory"
                );
                assert!(metadata.perm.contains("64"), "Permessi non corrispondono");

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
                println!("  🔒 Permessi: {}", metadata.perm);
                println!("  📅 Ultima modifica: {}", metadata.mtime);
                println!("  📂 È directory: {:?}", metadata.kind);

                // Verifica che sia marcata come directory
                assert!(
                    match metadata.kind {
                        FileKind::Directory => true,
                        _ => false,
                    },
                    "La directory dovrebbe essere marcata come directory"
                );
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
            size: content.len() as u64,
            data: Some(content.clone()),
            new_path: None,
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
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
                println!("  📂 È directory: {:?}", metadata.kind);

                assert_eq!(metadata.name, test_file, "Nome file nested non corrisponde");
                assert_eq!(
                    metadata.size,
                    content.len() as u64,
                    "Dimensione file nested non corrisponde"
                );
                assert!(
                    match metadata.kind {
                        FileKind::RegularFile => true,
                        _ => false,
                    },
                    "File nested non dovrebbe essere directory"
                );

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

        println!(
            "❌ TEST: Get metadata file inesistente {}",
            non_existent_file
        );

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
                        println!("  📂 È directory: {:?}", metadata.kind);

                        // Verifica che i metadati corrispondano a quelli del listing
                        assert_eq!(metadata.name, first_file.name);
                        assert_eq!(metadata.size, first_file.size);
                        assert!(
                            match metadata.kind {
                                FileKind::Directory => true,
                                FileKind::RegularFile => false,
                                FileKind::Symlink => false,
                                FileKind::Hardlink => false,
                            },
                            "Il file dovrebbe essere un file regolare"
                        );

                        println!("✅ VERIFICA: Metadati consistenti con il listing");
                    }
                    Err(e) => {
                        println!("❌ ERROR: {}", e);
                        assert!(
                            false,
                            "Fallimento get_file_metadata per file esistente: {}",
                            e
                        );
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

    #[tokio::test]
    async fn test_create_symbolic_link() {
        let client = create_test_client();

        let target_file = "/target_file_for_symlink.txt";
        let symlink_path = "/test_symbolic_link.txt";
        let content = "Contenuto del file target".as_bytes().to_vec();

        println!(
            "🔗 TEST: Creazione symbolic link da {} a {}",
            symlink_path, target_file
        );

        // 1. Crea il file target
        println!("📄 STEP 1: Creazione file target");
        let target_request = WriteRequest {
            path: target_file.to_string(),
            new_path: None,
            size: content.len() as u64,
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
            data: Some(content.clone()),
        };

        match client.write_file(&target_request).await {
            Ok(()) => println!("✅ SUCCESS: File target creato"),
            Err(e) => assert!(false, "Fallimento creazione file target: {}", e),
        }

        // 2. Crea il symbolic link
        println!("🔗 STEP 2: Creazione symbolic link");
        let symlink_request = WriteRequest {
            path: symlink_path.to_string(),
            new_path: None,
            size: 0,
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::Symlink, // ← Usa FileKind::Symlink
            ref_path: Some(target_file.to_string()),
            perm: "777".to_string(),
            mode: Mode::Write,
            data: None,
        };

        // ... rest of test
    }

    #[tokio::test]
    async fn test_write_append_mode() {
        let client = create_test_client();

        let test_file = "/test_append_file.txt";
        let initial_content = "Contenuto iniziale\n".as_bytes().to_vec();
        let append_content = "Contenuto aggiunto\n".as_bytes().to_vec();

        println!("📝 TEST: Modalità append su file {}", test_file);

        // 1. Crea file iniziale
        println!("📄 STEP 1: Creazione file iniziale");
        let initial_request = WriteRequest {
            path: test_file.to_string(),
            new_path: None,
            size: initial_content.len() as u64,
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Write,
            data: Some(initial_content.clone()),
        };

        // 2. Append contenuto
        println!("➕ STEP 2: Append contenuto al file");
        let append_request = WriteRequest {
            path: test_file.to_string(),
            new_path: None,
            size: append_content.len() as u64,
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Append, // ← Usa Mode::Append
            data: Some(append_content.clone()),
        };

        // ... rest of test
    }

    #[tokio::test]
    async fn test_write_at_mode() {
        let client = create_test_client();

        let test_file = "/test_write_at_file.txt";
        let initial_content = "0123456789ABCDEF".as_bytes().to_vec();
        let patch_content = "XXXX".as_bytes().to_vec();

        // Nota: WriteAt necessita di offset, ma non vedo questo campo in WriteRequest
        // Potrebbe essere necessario aggiungere offset: Option<u64> alla struct

        let write_at_request = WriteRequest {
            path: test_file.to_string(),
            new_path: None,
            size: patch_content.len() as u64,
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::WriteAt,
            data: Some(patch_content.clone()),
        };

        // ... rest of test
    }

    #[tokio::test]
    async fn test_truncate_mode() {
        let client = create_test_client();

        let test_file = "/test_truncate_file.txt";
        let new_size = 20u64;

        let truncate_request = WriteRequest {
            path: test_file.to_string(),
            new_path: None,
            size: new_size, // ← Per truncate, size è la dimensione finale
            atime: chrono::Utc::now().to_rfc3339(),
            mtime: chrono::Utc::now().to_rfc3339(),
            ctime: chrono::Utc::now().to_rfc3339(),
            crtime: chrono::Utc::now().to_rfc3339(),
            kind: FileKind::RegularFile,
            ref_path: None,
            perm: "644".to_string(),
            mode: Mode::Truncate,
            data: None, // ← Nessun contenuto per truncate
        };
    }
}
*/