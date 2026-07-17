use std::path::PathBuf;

use clap::{Parser, Subcommand};
use stitch::exec::{
    build_plan, build_scope, parse_closure_mode, parse_order_mode, ClosureMode, ExecutionMode,
    ExecutionScope, RunOptions, SelectionMode,
};

#[derive(Parser)]
#[command(
    name = "stitch",
    version,
    about = "Run commands across a repository DAG in a deterministic order"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    Graph {
        #[command(subcommand)]
        command: GraphCommand,
    },
    Status {
        #[arg(long)]
        dirty_only: bool,
        #[arg(long)]
        json: bool,
    },
    Exec(ExecArgs),
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    Discover {
        #[arg(default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum GraphCommand {
    Derive {
        #[arg(default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Verify {
        #[arg(default_value = ".")]
        workspace: PathBuf,
        #[arg(long)]
        strict: bool,
        #[arg(long)]
        json: bool,
    },
    Order {
        #[arg(default_value = ".")]
        workspace: PathBuf,
        #[arg(long, default_value = "providers-first")]
        order: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args)]
struct ExecArgs {
    #[arg(long, conflicts_with_all = ["changed", "dirty", "current", "node"])]
    all: bool,
    #[arg(long, conflicts_with_all = ["all", "dirty", "current", "node"])]
    changed: bool,
    #[arg(long, conflicts_with_all = ["all", "changed", "current", "node"])]
    dirty: bool,
    #[arg(long, conflicts_with_all = ["all", "changed", "dirty", "node"])]
    current: bool,
    #[arg(long, conflicts_with_all = ["all", "changed", "dirty", "current"])]
    node: Vec<String>,
    #[arg(long, default_value = "self")]
    closure: String,
    #[arg(long, default_value = "providers-first")]
    order: String,
    #[arg(long)]
    apply: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    keep_going: bool,
    #[arg(long)]
    json: bool,
    #[arg(last = true, required = true)]
    command: Vec<String>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    match Cli::parse().command {
        Command::Workspace { command } => workspace(command),
        Command::Graph { command } => graph(command),
        Command::Status { dirty_only, json } => status(dirty_only, json),
        Command::Exec(args) => exec(args),
    }
}

fn workspace(command: WorkspaceCommand) -> Result<(), String> {
    match command {
        WorkspaceCommand::Discover { workspace, json } => {
            let config = stitch::workspace::load_workspace_config(&workspace)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?
                );
            } else {
                for repository in config.repos {
                    println!("{}\t{}", repository.name, repository.path);
                }
            }
            Ok(())
        }
    }
}

fn graph(command: GraphCommand) -> Result<(), String> {
    match command {
        GraphCommand::Derive { workspace, json } => {
            let graph = stitch::graph::derive_workspace_graph(&workspace, None)
                .map_err(|error| error.to_string())?;
            print_graph(&graph, json)
        }
        GraphCommand::Verify {
            workspace,
            strict,
            json,
        } => {
            let graph = stitch::graph::derive_workspace_graph(&workspace, None)
                .map_err(|error| error.to_string())?;
            let report =
                stitch::graph::validate_graph(&graph, &stitch::graph::ValidateOptions { strict });
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
                );
            } else {
                for diagnostic in &report.diagnostics {
                    println!(
                        "{:?}\t{}\t{}",
                        diagnostic.severity, diagnostic.code, diagnostic.message
                    );
                }
            }
            if report.valid {
                Ok(())
            } else {
                Err("graph validation failed".to_string())
            }
        }
        GraphCommand::Order {
            workspace,
            order,
            json,
        } => {
            let config = stitch::workspace::load_workspace_config(&workspace)?;
            let scope = ExecutionScope {
                selection: SelectionMode::All,
                explicit_nodes: Vec::new(),
                closure: ClosureMode::All,
                order: parse_order_mode(&order)?,
            };
            let nodes = build_scope(&config, &scope)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&nodes).map_err(|error| error.to_string())?
                );
            } else {
                for node in nodes {
                    println!("{}", node.name);
                }
            }
            Ok(())
        }
    }
}

fn print_graph(graph: &stitch::graph::CanonicalWorkspaceGraph, json: bool) -> Result<(), String> {
    let nodes = graph
        .node_ids()
        .filter_map(|id| graph.node(id))
        .cloned()
        .collect::<Vec<_>>();
    let edges = graph
        .semantic_edges()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "nodes": nodes, "edges": edges }))
                .map_err(|error| error.to_string())?
        );
    } else {
        for edge in edges {
            println!("{} -> {}", edge.from, edge.to);
        }
    }
    Ok(())
}

fn status(dirty_only: bool, json: bool) -> Result<(), String> {
    let config = stitch::config::find_and_load()?;
    let statuses = stitch::status::collect_all(&config)?;
    let statuses = statuses
        .into_iter()
        .filter(|status| !dirty_only || status.is_dirty)
        .collect::<Vec<_>>();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&statuses).map_err(|error| error.to_string())?
        );
    } else {
        for status in statuses {
            println!(
                "{}\t{}\t{}",
                status.name,
                status.branch,
                if status.is_dirty { "dirty" } else { "clean" }
            );
        }
    }
    Ok(())
}

fn exec(args: ExecArgs) -> Result<(), String> {
    let config = stitch::config::find_and_load()?;
    let selection = if args.all {
        SelectionMode::All
    } else if args.changed {
        SelectionMode::Changed
    } else if args.dirty {
        SelectionMode::Dirty
    } else if args.current {
        SelectionMode::Current
    } else if !args.node.is_empty() {
        SelectionMode::Explicit
    } else {
        SelectionMode::Current
    };
    let scope = ExecutionScope {
        selection,
        explicit_nodes: args.node,
        closure: parse_closure_mode(&args.closure)?,
        order: parse_order_mode(&args.order)?,
    };
    let mode = if args.apply {
        ExecutionMode::Mutating
    } else {
        ExecutionMode::ReadOnly
    };
    let plan = build_plan(&config, &scope, args.command, mode)?;
    let report = stitch::exec::run_plan(
        &plan,
        &RunOptions {
            dry_run: args.dry_run,
            apply: args.apply,
            keep_going: args.keep_going,
        },
    )?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
        );
    } else {
        for result in &report.node_results {
            println!(
                "{}: {}",
                result.node,
                if result.success { "ok" } else { "failed" }
            );
            if !result.stdout.is_empty() {
                print!("{}", result.stdout);
            }
            if !result.stderr.is_empty() {
                eprint!("{}", result.stderr);
            }
        }
    }
    if report.failed_nodes == 0 {
        Ok(())
    } else {
        Err(format!(
            "{} repository command(s) failed",
            report.failed_nodes
        ))
    }
}
