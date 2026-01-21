use glob::Pattern;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FileFilter {
    pub extensions: Vec<String>,
    pub include_globs: Vec<Pattern>,
    pub exclude_globs: Vec<Pattern>,
}

impl FileFilter {
    pub fn new(
        extensions: Vec<String>,
        include_globs: Vec<String>,
        exclude_globs: Vec<String>,
    ) -> Self {
        let include_globs = include_globs
            .into_iter()
            .filter_map(|s| Pattern::new(&s).ok())
            .collect();
        let exclude_globs = exclude_globs
            .into_iter()
            .filter_map(|s| Pattern::new(&s).ok())
            .collect();

        Self {
            extensions,
            include_globs,
            exclude_globs,
        }
    }

    pub fn matches(&self, path: &Path) -> bool {
        // 1. Check extension (whitelist)
        if !self.extensions.is_empty() {
            match path.extension().and_then(|e| e.to_str()) {
                Some(ext) => {
                    if !self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                        return false;
                    }
                }
                None => return false,
            }
        }

        // 2. Check exclude patterns (blacklist)
        for pattern in &self.exclude_globs {
            if pattern.matches_path(path) {
                return false;
            }
        }

        // 3. Check include patterns (whitelist)
        if !self.include_globs.is_empty() {
            let mut matched = false;
            for pattern in &self.include_globs {
                if pattern.matches_path(path) {
                    matched = true;
                    break;
                }
            }
            if !matched {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // Note: glob matching is relative to CWD usually, but Pattern matches absolute paths too if they match the string.
        // glob::Pattern matches against the string representation.
        // "src/**/*.rs" will match "src/main.rs"
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

        // Must match extension AND include glob (if present) AND NOT exclude glob
        // Wait, logic implementation:
        // 1. Check extension (whitelist) - if present, MUST match
        // 2. Check exclude (blacklist) - if matches, return FALSE
        // 3. Check include (whitelist) - if present, MUST match

        // "src/main.rs" -> ext=rs (ok), exclude= (ok), include=src/** (ok) -> true
        assert!(filter.matches(&PathBuf::from("src/main.rs")));

        // "src/main.py" -> ext=rs (fail) -> false
        assert!(!filter.matches(&PathBuf::from("src/main.py")));

        // "tests/main.rs" -> ext=rs (ok), exclude= (ok), include=src/** (fail) -> false
        assert!(!filter.matches(&PathBuf::from("tests/main.rs")));

        // "src/my_test.rs" -> ext=rs (ok), exclude=*_test.rs (fail) -> false
        assert!(!filter.matches(&PathBuf::from("src/my_test.rs")));
    }
}

impl Default for FileFilter {
    fn default() -> Self {
        Self {
            extensions: Vec::new(),
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
        }
    }
}
