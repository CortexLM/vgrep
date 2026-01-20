use std::fs;
use std::path::Path;

pub(crate) fn should_index_path(path: &Path, max_file_size: u64) -> bool {
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.len() > max_file_size {
            return false;
        }
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    matches!(
        ext.as_str(),
        "rs" | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "go"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "java"
            | "kt"
            | "swift"
            | "rb"
            | "php"
            | "cs"
            | "fs"
            | "scala"
            | "clj"
            | "ex"
            | "exs"
            | "erl"
            | "hs"
            | "ml"
            | "lua"
            | "r"
            | "jl"
            | "dart"
            | "vue"
            | "svelte"
            | "astro"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "json"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "md"
            | "markdown"
            | "txt"
            | "rst"
            | "tex"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "bat"
            | "cmd"
            | "sql"
            | "graphql"
            | "proto"
    ) || path.file_name().is_some_and(|n| {
        let name = n.to_string_lossy().to_lowercase();
        matches!(
            name.as_str(),
            "dockerfile"
                | "makefile"
                | "cmakelists.txt"
                | "rakefile"
                | "gemfile"
                | "podfile"
                | "vagrantfile"
                | ".gitignore"
                | ".dockerignore"
                | ".env.example"
                | "readme"
                | "license"
                | "changelog"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_index_extensionless_files_by_default() {
        assert!(!should_index_path(Path::new("my_binary"), 512 * 1024));
    }

    #[test]
    fn indexes_known_extensionless_filenames() {
        assert!(should_index_path(Path::new("Makefile"), 512 * 1024));
        assert!(should_index_path(Path::new("Dockerfile"), 512 * 1024));
        assert!(should_index_path(Path::new("README"), 512 * 1024));
    }

    #[test]
    fn indexes_known_extensions() {
        assert!(should_index_path(Path::new("src/main.rs"), 512 * 1024));
        assert!(should_index_path(Path::new("script.SH"), 512 * 1024));
    }
}
