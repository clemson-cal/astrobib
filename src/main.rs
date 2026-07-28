use astrobib::library::{find_manuscript_db, MergedLibrary};
use astrobib::query::{self, QueryContext};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "astrobib", version, about = "ADS-native BibTeX library manager (Rust port)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// List library entries, newest first
    List {
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
    },
    /// Search the local library with the filter language
    Search {
        query: String,
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
    },
    /// Print the BibTeX entry for a cite key (full or shortened)
    Show { key: String },
    /// Star an entry (personal library only); --off unstars
    Star {
        key: String,
        #[arg(long)]
        off: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let ms_root = find_manuscript_db();
    let mut lib = MergedLibrary::load(ms_root.as_deref())?;
    match cli.command {
        None => astrobib::tui::run(lib),
        Some(Command::List { limit }) => {
            print_entries(&lib, |_| true, limit);
            Ok(())
        }
        Some(Command::Search { query, limit }) => {
            let groups = query::tokenize(&query);
            let ctx = QueryContext::default();
            print_entries(&lib, |e| query::matches(&groups, e, &ctx), limit);
            Ok(())
        }
        Some(Command::Show { key }) => match lib.resolve(&key) {
            Some(e) => {
                print!("{}", std::fs::read_to_string(&e.path)?);
                Ok(())
            }
            None => {
                let matches = lib.possible_matches(&key);
                if matches.len() > 1 {
                    eprintln!("Ambiguous key '{key}' — did you mean:");
                    for m in matches.iter().take(6) {
                        eprintln!("  {}  {}", m.short_key, truncate(m.title(), 55));
                    }
                } else {
                    eprintln!("{key} not found.");
                }
                std::process::exit(1);
            }
        },
        Some(Command::Star { key, off }) => {
            let Some(full) = lib.resolve(&key).map(|e| e.key().to_string()) else {
                eprintln!("{key} not found.");
                std::process::exit(1);
            };
            lib.set_starred(&full, !off)?;
            println!("{} {full}", if off { "Unstarred" } else { "★ Starred" });
            Ok(())
        }
    }
}

fn print_entries(
    lib: &MergedLibrary,
    pred: impl Fn(&astrobib::library::Entry) -> bool,
    limit: usize,
) {
    let mut shown: Vec<_> = lib.entries().into_iter().filter(|e| pred(e)).collect();
    shown.sort_by(|a, b| b.year().cmp(&a.year()).then(a.key().cmp(b.key())));
    for e in shown.iter().take(limit) {
        println!(
            "{:<24} {:<6} {:<18} {}",
            e.short_key,
            e.year(),
            truncate(e.first_author_last().trim_start_matches('{'), 18),
            truncate(e.title().trim_matches(['{', '}']), 60),
        );
    }
    println!("{} result(s)", shown.len().min(limit));
}

fn truncate(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        let cut: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}
