//! Validate command - check agent syntax and security

use crate::print_banner;
use crate::style::*;
use anyhow::{anyhow, Result};
use std::path::PathBuf;

const FORBIDDEN_IMPORTS: [&str; 10] = [
    "subprocess",
    "os.system",
    "os.popen",
    "os.exec",
    "commands",
    "pty",
    "socket",
    "ctypes",
    "pickle",
    "marshal",
];

const FORBIDDEN_BUILTINS: [&str; 5] = ["exec(", "eval(", "compile(", "__import__(", "open("];

pub async fn run(agent: PathBuf) -> Result<()> {
    print_banner();
    print_header("Agent Validation");

    // Check file exists
    if !agent.exists() {
        return Err(anyhow!("File not found: {}", agent.display()));
    }

    let filename = agent
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    println!(
        "  {} Validating {}{}{}",
        icon_arrow(),
        BOLD,
        filename,
        RESET
    );
    println!();

    // Read source
    let source = std::fs::read_to_string(&agent)?;
    let lines: Vec<&str> = source.lines().collect();

    print_key_value("File", &agent.display().to_string());
    print_key_value("Size", &format!("{} bytes", source.len()));
    print_key_value("Lines", &format!("{}", lines.len()));
    println!();

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Check Python extension
    print_step(1, 5, "Checking file type...");
    if !filename.ends_with(".py") {
        warnings.push("File does not have .py extension".to_string());
    }

    // Check for forbidden imports
    print_step(2, 5, "Checking for forbidden imports...");
    for forbidden in FORBIDDEN_IMPORTS {
        if source.contains(&format!("import {}", forbidden))
            || source.contains(&format!("from {} import", forbidden))
        {
            errors.push(format!("Forbidden import: {}", forbidden));
        }
    }

    // Check for forbidden builtins
    print_step(3, 5, "Checking for dangerous builtins...");
    for forbidden in FORBIDDEN_BUILTINS {
        if source.contains(forbidden) {
            errors.push(format!(
                "Forbidden builtin: {}",
                forbidden.trim_end_matches('(')
            ));
        }
    }

    // Check for required structure (term_sdk format)
    print_step(4, 5, "Checking code structure...");

    // Check for term_sdk import
    let has_sdk_import = source.contains("from term_sdk import")
        || source.contains("import term_sdk")
        || source.contains("from termsdk import")
        || source.contains("import termsdk");

    if !has_sdk_import {
        warnings.push(
            "No term_sdk import found (expected: from term_sdk import Agent, ...)".to_string(),
        );
    }

    // Check for Agent class extending base
    let has_agent_class =
        source.contains("class ") && (source.contains("(Agent)") || source.contains("( Agent )"));

    if !has_agent_class {
        warnings.push("No Agent class found (expected: class MyAgent(Agent):)".to_string());
    }

    // Check for run() method
    let has_run_method = source.contains("def run") || source.contains("async def run");

    if !has_run_method {
        errors.push("Missing run() method - required by term_sdk 2.0".to_string());
    }

    // Check for run() entry point
    let has_run = source.contains("run(") && source.contains("if __name__");

    if !has_run {
        warnings.push(
            "No run() entry point (expected: if __name__ == '__main__': run(MyAgent()))"
                .to_string(),
        );
    }

    // Check encoding
    print_step(5, 5, "Checking encoding...");
    if source
        .chars()
        .any(|c| !c.is_ascii() && !c.is_alphanumeric())
    {
        warnings.push("File contains non-ASCII characters".to_string());
    }

    println!();

    // Print results
    if errors.is_empty() && warnings.is_empty() {
        print_box(
            "Validation Result",
            &[
                &format!("{} All checks passed!", icon_success()),
                "",
                "Your agent is ready to submit.",
                &format!("Run: {} submit -a {}", style_cyan("term"), filename),
            ],
        );
    } else {
        if !errors.is_empty() {
            print_section("Errors");
            for error in &errors {
                println!("    {} {}", icon_error(), style_red(error));
            }
        }

        if !warnings.is_empty() {
            print_section("Warnings");
            for warning in &warnings {
                println!("    {} {}", icon_warning(), style_yellow(warning));
            }
        }

        println!();

        if !errors.is_empty() {
            print_error("Validation failed. Please fix the errors above.");
            return Err(anyhow!("Validation failed with {} error(s)", errors.len()));
        } else {
            print_warning("Validation passed with warnings.");
        }
    }

    println!();
    Ok(())
}

use crate::style::colors::*;
