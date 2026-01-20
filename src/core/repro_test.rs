
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_path_prefix_search_bug_reproduction() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("test.db");
        let db = Database::new(&db_path)?;

        // Setup paths
        let base_path = PathBuf::from("/home/user");
        let path1 = base_path.join("proj/file1.txt");
        let path2 = base_path.join("project/file2.txt");

        // Insert files
        let id1 = db.insert_file(&path1, "hash1")?;
        let id2 = db.insert_file(&path2, "hash2")?;

        // Insert dummy chunks
        let embedding = vec![0.1; 128];
        db.insert_chunk(id1, 0, "content1", 1, 1, &embedding)?;
        db.insert_chunk(id2, 0, "content2", 1, 1, &embedding)?;

        // Search with prefix "/home/user/proj"
        // This should match ONLY "/home/user/proj/file1.txt"
        // But due to the bug, it likely matches "/home/user/project/file2.txt" as well
        let prefix = base_path.join("proj");
        let results = db.search_similar(&embedding, &prefix, 10)?;

        // If bug exists, we might get 2 results instead of 1
        // Or specific result path matching expectations
        
        let found_paths: Vec<String> = results.iter()
            .map(|r| r.path.to_string_lossy().to_string())
            .collect();
            
        println!("Found paths: {:?}", found_paths);
        
        // Assert that we DO NOT match "project" when searching for "proj"
        let matched_project = found_paths.iter().any(|p| p.contains("project"));
        assert!(!matched_project, "Should not match sibling directory 'project' when searching for 'proj'");
        
        Ok(())
    }
}
