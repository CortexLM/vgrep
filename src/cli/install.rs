use crate::utils::write_atomic;
use anyhow::{Context, Result};
use console::style;
use std::fs;
use std::path::PathBuf;

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("Could not determine home directory")
}

const VGREP_SKILL: &str = r#"
---
name: vgrep
description: A local semantic code search tool. Uses llama.cpp embeddings to find code by meaning, not just keywords. Much better than grep for understanding code intent.
license: Apache 2.0
---

## When to use this skill

Use vgrep whenever you need to search code semantically. It understands intent and finds related code even without exact keyword matches.

## How to use this skill

Use `vgrep` to search your local files. The search is semantic - describe what you are looking for in natural language.

### Examples

```bash
vgrep "where is authentication handled?"
vgrep "database connection logic"
vgrep "error handling for network requests"
vgrep -m 20 "how are users validated"
vgrep -c "payment processing" # show content snippets
```

### Setup

1. Start the vgrep server: `vgrep serve`
2. Index your code: `vgrep watch`
3. Search: `vgrep "your query"`

## Keywords
search, grep, semantic, code search, local search, embeddings, llm
"#;

pub fn install_claude_code() -> Result<()> {
    println!(
        "  {} Installing vgrep for Claude Code...",
        style(">>").cyan()
    );

    let claude_dir = home_dir()?.join(".claude");
    fs::create_dir_all(&claude_dir)?;

    let instructions_path = claude_dir.join("CLAUDE.md");
    let mut content = if instructions_path.exists() {
        fs::read_to_string(&instructions_path)?
    } else {
        String::new()
    };

    if !content.contains("vgrep") {
        content.push_str("\n\n");
        content.push_str(VGREP_SKILL.trim());
        write_atomic(&instructions_path, content)?;
        println!(
            "  {} Added vgrep skill to Claude Code",
            style("[+]").green()
        );
    } else {
        println!(
            "  {} vgrep already configured for Claude Code",
            style("[=]").yellow()
        );
    }

    println!();
    println!("  {} Setup complete!", style("OK").green().bold());
    println!();
    println!("  Next steps:");
    println!("    1. Start vgrep server: {}", style("vgrep serve").cyan());
    println!(
        "    2. In another terminal: {}",
        style("vgrep watch").cyan()
    );
    println!("    3. Claude Code can now use vgrep for semantic search");
    println!();

    Ok(())
}

pub fn uninstall_claude_code() -> Result<()> {
    println!(
        "  {} Uninstalling vgrep from Claude Code...",
        style(">>").cyan()
    );

    let instructions_path = home_dir()?.join(".claude").join("CLAUDE.md");

    if instructions_path.exists() {
        let content = fs::read_to_string(&instructions_path)?;
        let updated = content
            .replace(VGREP_SKILL.trim(), "")
            .replace("\n\n\n", "\n\n");
        write_atomic(&instructions_path, updated.trim())?;
        println!("  {} Removed vgrep from Claude Code", style("[-]").green());
    } else {
        println!(
            "  {} vgrep not installed for Claude Code",
            style("[=]").yellow()
        );
    }

    Ok(())
}

pub fn install_opencode() -> Result<()> {
    println!("  {} Installing vgrep for OpenCode...", style(">>").cyan());

    let config_dir = home_dir()?.join(".config").join("opencode");
    fs::create_dir_all(&config_dir)?;

    // Create tool definition
    let tool_dir = config_dir.join("tool");
    fs::create_dir_all(&tool_dir)?;

    let tool_content = r#"
import { tool } from "@opencode-ai/plugin"

export default tool({
  description: `A local semantic code search tool. Uses llama.cpp embeddings to find code by meaning.
  
Usage:
  vgrep "your search query"
  vgrep -m 20 "query"  # limit results
  vgrep -c "query"     # show content`,
  args: {
    q: tool.schema.string().describe("The semantic search query."),
    m: tool.schema.number().default(10).describe("Max results to return."),
  },
  async execute(args) {
    const result = await Bun.$`vgrep -m ${args.m} ${args.q}`.text()
    return result.trim()
  },
})"#;

    let tool_path = tool_dir.join("vgrep.ts");
    write_atomic(&tool_path, tool_content.trim())?;
    println!("  {} Created vgrep tool", style("[+]").green());

    // Update opencode.json
    let config_path = config_dir.join("opencode.json");
    let mut config: serde_json::Value = if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if config.get("$schema").is_none() {
        config["$schema"] = serde_json::json!("https://opencode.ai/config.json");
    }
    if config.get("mcp").is_none() {
        config["mcp"] = serde_json::json!({});
    }

    config["mcp"]["vgrep"] = serde_json::json!({
        "type": "local",
        "command": ["vgrep", "serve"],
        "enabled": true
    });

    write_atomic(&config_path, serde_json::to_string_pretty(&config)?)?;
    println!("  {} Updated OpenCode config", style("[+]").green());

    println!();
    println!("  {} Setup complete!", style("OK").green().bold());
    println!();

    Ok(())
}

pub fn uninstall_opencode() -> Result<()> {
    println!(
        "  {} Uninstalling vgrep from OpenCode...",
        style(">>").cyan()
    );

    let config_dir = home_dir()?.join(".config").join("opencode");

    // Remove tool
    let tool_path = config_dir.join("tool").join("vgrep.ts");
    if tool_path.exists() {
        fs::remove_file(&tool_path)?;
        println!("  {} Removed vgrep tool", style("[-]").green());
    }

    // Update config
    let config_path = config_dir.join("opencode.json");
    if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        if let Ok(mut config) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(mcp) = config.get_mut("mcp") {
                if let Some(obj) = mcp.as_object_mut() {
                    obj.remove("vgrep");
                }
            }
            write_atomic(&config_path, serde_json::to_string_pretty(&config)?)?;
            println!("  {} Updated OpenCode config", style("[-]").green());
        }
    }

    Ok(())
}

pub fn install_codex() -> Result<()> {
    println!("  {} Installing vgrep for Codex...", style(">>").cyan());

    let codex_dir = home_dir()?.join(".codex");
    fs::create_dir_all(&codex_dir)?;

    let agents_path = codex_dir.join("AGENTS.md");
    let mut content = if agents_path.exists() {
        fs::read_to_string(&agents_path)?
    } else {
        String::new()
    };

    if !content.contains("vgrep") {
        content.push_str("\n\n");
        content.push_str(VGREP_SKILL.trim());
        write_atomic(&agents_path, content)?;
        println!("  {} Added vgrep skill to Codex", style("[+]").green());
    } else {
        println!(
            "  {} vgrep already configured for Codex",
            style("[=]").yellow()
        );
    }

    println!();
    println!("  {} Setup complete!", style("OK").green().bold());
    println!();

    Ok(())
}

pub fn uninstall_codex() -> Result<()> {
    println!("  {} Uninstalling vgrep from Codex...", style(">>").cyan());

    let agents_path = home_dir()?.join(".codex").join("AGENTS.md");

    if agents_path.exists() {
        let content = fs::read_to_string(&agents_path)?;
        let updated = content
            .replace(VGREP_SKILL.trim(), "")
            .replace("\n\n\n", "\n\n");
        if updated.trim().is_empty() {
            fs::remove_file(&agents_path)?;
        } else {
            write_atomic(&agents_path, updated.trim())?;
        }
        println!("  {} Removed vgrep from Codex", style("[-]").green());
    }

    Ok(())
}

pub fn install_droid() -> Result<()> {
    println!(
        "  {} Installing vgrep for Factory Droid...",
        style(">>").cyan()
    );

    let factory_dir = home_dir()?.join(".factory");
    if !factory_dir.exists() {
        anyhow::bail!(
            "Factory Droid directory not found at {}. Start Factory Droid once first.",
            factory_dir.display()
        );
    }

    // Create skill
    let skills_dir = factory_dir.join("skills").join("vgrep");
    fs::create_dir_all(&skills_dir)?;

    let skill_content = VGREP_SKILL.trim();
    write_atomic(skills_dir.join("SKILL.md"), skill_content)?;
    println!("  {} Created vgrep skill", style("[+]").green());

    // Create hooks
    let hooks_dir = factory_dir.join("hooks").join("vgrep");
    fs::create_dir_all(&hooks_dir)?;

    let watch_hook = r#"#!/usr/bin/env python3
"""Start vgrep watch on session start."""
import subprocess
import os
import sys

def main():
    try:
        # Check if vgrep server is running
        result = subprocess.run(
            ["vgrep", "status"],
            capture_output=True,
            text=True,
            timeout=5
        )
        if "Not running" in result.stdout:
            # Start vgrep watch in background
            subprocess.Popen(
                ["vgrep", "watch"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True
            )
    except Exception as e:
        pass  # Silently fail

if __name__ == "__main__":
    main()
"#;

    let kill_hook = r#"#!/usr/bin/env python3
"""Stop vgrep watch on session end."""
import subprocess
import os
import signal
import sys

def main():
    try:
        # Find and kill vgrep watch processes
        if sys.platform == "win32":
            subprocess.run(["taskkill", "/F", "/IM", "vgrep.exe"], 
                         capture_output=True)
        else:
            subprocess.run(["pkill", "-f", "vgrep watch"], 
                         capture_output=True)
    except Exception:
        pass

if __name__ == "__main__":
    main()
"#;

    write_atomic(hooks_dir.join("vgrep_watch.py"), watch_hook)?;
    write_atomic(hooks_dir.join("vgrep_watch_kill.py"), kill_hook)?;
    println!("  {} Created vgrep hooks", style("[+]").green());

    // Update settings.json
    let settings_path = factory_dir.join("settings.json");
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    settings["enableHooks"] = serde_json::json!(true);
    settings["allowBackgroundProcesses"] = serde_json::json!(true);

    let watch_py = hooks_dir.join("vgrep_watch.py");
    let kill_py = hooks_dir.join("vgrep_watch_kill.py");

    let hooks = settings
        .get_mut("hooks")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let mut hooks = hooks.as_object().cloned().unwrap_or_default();

    // Add SessionStart hook
    let session_start = hooks
        .entry("SessionStart".to_string())
        .or_insert(serde_json::json!([]))
        .as_array_mut();

    if let Some(arr) = session_start {
        let hook_entry = serde_json::json!({
            "matcher": "startup|resume",
            "hooks": [{
                "type": "command",
                "command": format!("python3 \"{}\"", watch_py.display()),
                "timeout": 10
            }]
        });
        if !arr.iter().any(|h| h.to_string().contains("vgrep_watch.py")) {
            arr.push(hook_entry);
        }
    }

    // Add SessionEnd hook
    let session_end = hooks
        .entry("SessionEnd".to_string())
        .or_insert(serde_json::json!([]))
        .as_array_mut();

    if let Some(arr) = session_end {
        let hook_entry = serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": format!("python3 \"{}\"", kill_py.display()),
                "timeout": 10
            }]
        });
        if !arr
            .iter()
            .any(|h| h.to_string().contains("vgrep_watch_kill.py"))
        {
            arr.push(hook_entry);
        }
    }

    settings["hooks"] = serde_json::Value::Object(hooks);
    write_atomic(&settings_path, serde_json::to_string_pretty(&settings)?)?;
    println!("  {} Updated Factory Droid settings", style("[+]").green());

    println!();
    println!("  {} Setup complete!", style("OK").green().bold());
    println!();
    println!("  vgrep will auto-start when you begin a Droid session.");
    println!();

    Ok(())
}

pub fn uninstall_droid() -> Result<()> {
    println!(
        "  {} Uninstalling vgrep from Factory Droid...",
        style(">>").cyan()
    );

    let factory_dir = home_dir()?.join(".factory");

    // Remove skill
    let skills_dir = factory_dir.join("skills").join("vgrep");
    if skills_dir.exists() {
        fs::remove_dir_all(&skills_dir)?;
        println!("  {} Removed vgrep skill", style("[-]").green());
    }

    // Remove hooks
    let hooks_dir = factory_dir.join("hooks").join("vgrep");
    if hooks_dir.exists() {
        fs::remove_dir_all(&hooks_dir)?;
        println!("  {} Removed vgrep hooks", style("[-]").green());
    }

    // Clean settings.json
    let settings_path = factory_dir.join("settings.json");
    if settings_path.exists() {
        if let Ok(content) = fs::read_to_string(&settings_path) {
            if let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(hooks) = settings.get_mut("hooks") {
                    if let Some(obj) = hooks.as_object_mut() {
                        for (_, entries) in obj.iter_mut() {
                            if let Some(arr) = entries.as_array_mut() {
                                arr.retain(|h| !h.to_string().contains("vgrep"));
                            }
                        }
                        obj.retain(|_, v| v.as_array().map(|a| !a.is_empty()).unwrap_or(true));
                    }
                }
                write_atomic(&settings_path, serde_json::to_string_pretty(&settings)?)?;
            }
        }
        println!("  {} Updated Factory Droid settings", style("[-]").green());
    }

    Ok(())
}
