#[cfg(test)]
mod tests {
    use super::*;
    use glob::Pattern;
    use std::path::PathBuf;

    #[test]
    fn test_filter_extension() {
        let filter = FileFilter::new(
            vec!["rs".to_string(), "toml".to_string()],
            vec![],
            vec![],
        );

        assert!(filter.matches(&PathBuf::from("src/main.rs")));
        assert!(filter.matches(&PathBuf::from("Cargo.toml")));
        assert!(!filter.matches(&PathBuf::from("README.md")));
        assert!(!filter.matches(&PathBuf::from("src/main")));
    }

    #[test]
    fn test_filter_include_glob() {
        let filter = FileFilter::new(
            vec![],
            vec!["src/**/*.rs".to_string()],
            vec![],
        );

        assert!(filter.matches(&PathBuf::from("src/main.rs")));
        assert!(filter.matches(&PathBuf::from("src/core/mod.rs")));
        assert!(!filter.matches(&PathBuf::from("tests/main.rs")));
        assert!(!filter.matches(&PathBuf::from("README.md")));
    }

    #[test]
    fn test_filter_exclude_glob() {
        let filter = FileFilter::new(
            vec![],
            vec![],
            vec!["target/**".to_string(), "node_modules/**".to_string()],
        );

        assert!(filter.matches(&PathBuf::from("src/main.rs")));
        assert!(!filter.matches(&PathBuf::from("target/debug/vgrep")));
        assert!(!filter.matches(&PathBuf::from("node_modules/react/index.js")));
    }

    #[test]
    fn test_filter_combined() {
        let filter = FileFilter::new(
            vec!["rs".to_string()],
            vec!["src/**".to_string()],
            vec!["**/*_test.rs".to_string()],
        );

        // Must match extension AND include glob AND NOT exclude glob
        
        // "src/main.rs" -> ext=rs (ok), include=src/** (ok), exclude= (ok) -> true
        assert!(filter.matches(&PathBuf::from("src/main.rs")));

        // "src/main.py" -> ext=rs (fail) -> false
        assert!(!filter.matches(&PathBuf::from("src/main.py")));

        // "tests/main.rs" -> ext=rs (ok), include=src/** (fail) -> false
        assert!(!filter.matches(&PathBuf::from("tests/main.rs")));

        // "src/unit_test.rs" -> ext=rs (ok), include=src/** (ok), exclude=*_test.rs (fail) -> false
        assert!(!filter.matches(&PathBuf::from("src/unit_test.rs")));
    }
}
