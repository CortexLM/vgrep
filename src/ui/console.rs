use console::{style, Emoji, Term};

pub static SEARCH: Emoji<'_, '_> = Emoji("🔍 ", "");
pub static ROCKET: Emoji<'_, '_> = Emoji("🚀 ", "");
pub static CHECK: Emoji<'_, '_> = Emoji("✅ ", "[OK] ");
pub static CROSS: Emoji<'_, '_> = Emoji("❌ ", "[ERR] ");
pub static WARN: Emoji<'_, '_> = Emoji("⚠️  ", "[!] ");
pub static FOLDER: Emoji<'_, '_> = Emoji("📁 ", "");
pub static FILE: Emoji<'_, '_> = Emoji("📄 ", "");
pub static GEAR: Emoji<'_, '_> = Emoji("⚙️  ", "");
pub static DOWNLOAD: Emoji<'_, '_> = Emoji("📥 ", "");
pub static SERVER: Emoji<'_, '_> = Emoji("🖥️  ", "");
pub static EYE: Emoji<'_, '_> = Emoji("👁️  ", "");
pub static SPARKLES: Emoji<'_, '_> = Emoji("✨ ", "");
pub static BRAIN: Emoji<'_, '_> = Emoji("🧠 ", "");

/// Create a clickable hyperlink using OSC 8 escape sequence.
/// Falls back to plain text display in unsupporting terminals.
pub fn hyperlink(url: &str, text: &str) -> String {
    format!("\x1b]8;;{}\x07{}\x1b]8;;\x07", url, text)
}

/// Create a hyperlink where the URL is also the display text.
pub fn hyperlink_url(url: &str) -> String {
    hyperlink(url, url)
}

pub fn print_banner() {
    let banner = r#"
                                
 ██╗   ██╗ ██████╗ ██████╗ ███████╗██████╗ 
 ██║   ██║██╔════╝ ██╔══██╗██╔════╝██╔══██╗
 ██║   ██║██║  ███╗██████╔╝█████╗  ██████╔╝
 ╚██╗ ██╔╝██║   ██║██╔══██╗██╔══╝  ██╔═══╝ 
  ╚████╔╝ ╚██████╔╝██║  ██║███████╗██║     
   ╚═══╝   ╚═════╝ ╚═╝  ╚═╝╚══════╝╚═╝     
                                            
"#;
    println!("{}", style(banner).cyan().bold());
    println!(
        "  {} Local Semantic Search powered by {}",
        style("●").green(),
        style("Corτex Foundaτion").green().bold()
    );
    println!();
}

pub fn print_server_banner(host: &str, port: u16) {
    print_banner();
    println!(
        "  {}Server listening on {}",
        SERVER,
        style(format!("http://{}:{}", host, port)).green().bold()
    );
    println!();
    println!("  {}Endpoints:", GEAR);
    println!("    {} GET  /health   - Health check", style("•").dim());
    println!("    {} GET  /status   - Index status", style("•").dim());
    println!("    {} POST /search   - Semantic search", style("•").dim());
    println!(
        "    {} POST /embed    - Generate embeddings",
        style("•").dim()
    );
    println!();
    println!(
        "  {}Press {} to stop",
        style("→").dim(),
        style("Ctrl+C").yellow().bold()
    );
    println!();
    println!("{}", style("─".repeat(50)).dim());
    println!();
}

pub fn print_success(msg: &str) {
    println!("{}{}", CHECK, style(msg).green());
}

pub fn print_error(msg: &str) {
    println!("{}{}", CROSS, style(msg).red());
}

pub fn print_warning(msg: &str) {
    println!("{}{}", WARN, style(msg).yellow());
}

pub fn print_info(msg: &str) {
    println!("  {} {}", style("→").dim(), msg);
}

pub fn print_header(msg: &str) {
    println!();
    println!("{}", style(msg).cyan().bold());
    println!("{}", style("─".repeat(msg.len())).dim());
}

pub fn print_key_value(key: &str, value: &str) {
    println!(
        "  {} {} {}",
        style(format!("{:>16}:", key)).dim(),
        style("│").dim(),
        value
    );
}

pub fn print_key_value_colored(key: &str, value: &str, good: bool) {
    let colored_value = if good {
        style(value).green().to_string()
    } else {
        style(value).red().to_string()
    };
    println!(
        "  {} {} {}",
        style(format!("{:>16}:", key)).dim(),
        style("│").dim(),
        colored_value
    );
}

pub fn print_search_result(path: &str, score: &str, index: usize) {
    println!(
        "  {} {} {}",
        style(format!("{:>3}.", index + 1)).dim(),
        style(path).cyan(),
        style(format!("({})", score)).yellow()
    );
}

pub fn print_search_preview(content: &str) {
    for line in content.lines().take(3) {
        println!("      {}", style(line).dim());
    }
}

pub fn clear_screen() {
    let _ = Term::stdout().clear_screen();
}

pub fn print_section(title: &str) {
    println!();
    println!("  {}", style(format!("▸ {}", title)).bold());
}
