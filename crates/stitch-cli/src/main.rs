#![allow(clippy::items_after_test_module, clippy::too_many_arguments)]

use std::collections::BTreeMap;
use std::path::Path;

use clap::{Parser, Subcommand};

use stitch::config;
use stitch::exec;
use stitch::git;
use stitch::recipe;
use stitch::sync;

fn main() {
    let cli = Cli::parse();

    let result = match &cli.command {
        Commands::Repos => cmd_repos(),
        Commands::Status {
            json,
            short,
            dirty_only,
            repo,
        } => cmd_status(*json, *short, *dirty_only, repo.as_deref()),
        Commands::Diff { repo, staged, json } => cmd_diff(repo.as_deref(), *staged, *json),
        Commands::Dag { mode, split, json } => cmd_dag(mode.as_deref(), split.as_deref(), *json),
        Commands::Commit {
            dry_run,
            json: json_output,
            apply,
            force,
            resume,
            messages,
            write_template,
            message,
            repo,
        } => cmd_commit(
            *dry_run,
            *json_output,
            *apply,
            *force,
            resume.as_deref(),
            messages.as_deref(),
            *write_template,
            message.clone(),
            repo.clone(),
        ),
        Commands::Push {
            dry_run,
            json: json_output,
        } => cmd_push(*dry_run, *json_output),
        Commands::Sync {
            mode,
            apply,
            dry_run,
            json: json_output,
            no_push,
            repos,
            run_tend,
        } => cmd_sync(
            mode.as_deref(),
            *apply,
            *dry_run,
            *json_output,
            *no_push,
            repos,
            *run_tend,
        ),
        Commands::Graph { command } => cmd_graph(command),
        Commands::Topology { command } => cmd_topology(command),
        Commands::Hooks { command } => cmd_hooks(command),
        Commands::Exec {
            all: all_flag,
            changed,
            dirty,
            node,
            nodes,
            closure,
            order,
            mode,
            step,
            dry_run,
            apply,
            json,
            trailing_command,
        } => cmd_exec(
            *all_flag,
            *changed,
            *dirty,
            node.as_deref(),
            nodes,
            closure,
            order,
            mode,
            step,
            *dry_run,
            *apply,
            *json,
            trailing_command,
        ),
        Commands::Verify {
            node,
            nodes,
            all: all_flag,
            changed,
            dirty,
            upstream,
            downstream,
            dry_run,
            json,
        } => cmd_verify(
            node.as_deref(),
            nodes,
            *all_flag,
            *changed,
            *dirty,
            *upstream,
            *downstream,
            *dry_run,
            *json,
        ),
        Commands::Recipe { command } => cmd_recipe(command),
        Commands::Changeset { command } => cmd_changeset(command),
    };

    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_graph(command: &GraphCliCommand) -> Result<(), String> {
    match command {
        GraphCliCommand::Derive {
            workspace,
            source: _source,
            metadata,
            format,
        } => {
            let root = std::path::Path::new(workspace);
            let meta_path = metadata
                .as_ref()
                .map(|p| std::path::Path::new(p).to_path_buf());
            let meta_ref = meta_path.as_deref();

            let graph = stitch::graph::derive::derive_graph_from_locks(root, meta_ref)
                .map_err(|e| format!("Graph derivation failed: {e}"))?;

            let fmt = parse_format(format);
            let output = stitch::graph::render::render_graph_derive(&graph, fmt)?;
            println!("{output}");
            Ok(())
        }
        GraphCliCommand::Verify {
            workspace,
            source: _source,
            metadata,
            strict,
            format,
        } => {
            let root = std::path::Path::new(workspace);
            let meta_path = metadata
                .as_ref()
                .map(|p| std::path::Path::new(p).to_path_buf());
            let meta_ref = meta_path.as_deref();

            let graph = stitch::graph::derive::derive_graph_from_locks(root, meta_ref)
                .map_err(|e| format!("Graph derivation failed: {e}"))?;

            let opts = stitch::graph::ValidateOptions { strict: *strict };
            let report = stitch::graph::validate::validate_graph(&graph, &opts);
            let fmt = parse_format(format);
            let output = stitch::graph::render::render_validation_report(&report, fmt)?;
            println!("{output}");

            if !report.valid {
                Err("Graph validation failed".to_string())
            } else {
                Ok(())
            }
        }
        GraphCliCommand::Order {
            workspace,
            source: _source,
            metadata,
            format,
        } => {
            let root = std::path::Path::new(workspace);
            let meta_path = metadata
                .as_ref()
                .map(|p| std::path::Path::new(p).to_path_buf());
            let meta_ref = meta_path.as_deref();

            let graph = stitch::graph::derive::derive_graph_from_locks(root, meta_ref)
                .map_err(|e| format!("Graph derivation failed: {e}"))?;

            let order = stitch::graph::topo::provider_before_consumer_order(&graph)
                .map_err(|e| format!("Topological sort failed: {e}"))?;

            let fmt = parse_format(format);
            let output = stitch::graph::render::render_order(&graph, &order, fmt)?;
            println!("{output}");
            Ok(())
        }
        GraphCliCommand::Print {
            workspace,
            source: _source,
            metadata,
            format,
        } => {
            let root = std::path::Path::new(workspace);
            let meta_path = metadata
                .as_ref()
                .map(|p| std::path::Path::new(p).to_path_buf());
            let meta_ref = meta_path.as_deref();

            let graph = stitch::graph::derive::derive_graph_from_locks(root, meta_ref)
                .map_err(|e| format!("Graph derivation failed: {e}"))?;

            let fmt = parse_format(format);
            let output = stitch::graph::render::render_graph_derive(&graph, fmt)?;
            println!("{output}");
            Ok(())
        }
    }
}

fn parse_format(s: &str) -> stitch::graph::RenderFormat {
    match s {
        "json" => stitch::graph::RenderFormat::Json,
        "mermaid" => stitch::graph::RenderFormat::Mermaid,
        _ => stitch::graph::RenderFormat::Text,
    }
}

fn cmd_topology(command: &TopologyCommand) -> Result<(), String> {
    match command {
        TopologyCommand::Check {
            workspace,
            config,
            format,
        } => {
            let root = std::path::Path::new(workspace);
            let config_path = std::path::Path::new(config);

            let graph = stitch::graph::derive::derive_graph_from_locks(root, Some(config_path))
                .map_err(|e| format!("Topology derivation failed: {e}"))?;

            let report = stitch::graph::validate::validate_graph(
                &graph,
                &stitch::graph::ValidateOptions { strict: true },
            );

            let fmt = parse_format(format);
            let output = stitch::graph::render::render_validation_report(&report, fmt)?;
            println!("{output}");

            if report.valid {
                Ok(())
            } else {
                Err("Topology validation failed".to_string())
            }
        }
        TopologyCommand::Graph {
            workspace,
            config,
            format,
        } => {
            let root = std::path::Path::new(workspace);
            let config_path = std::path::Path::new(config);

            let graph = stitch::graph::derive::derive_graph_from_locks(root, Some(config_path))
                .map_err(|e| format!("Topology derivation failed: {e}"))?;

            let fmt = parse_format(format);
            let output = stitch::graph::render::render_graph_derive(&graph, fmt)?;
            println!("{output}");
            Ok(())
        }
    }
}

fn cmd_hooks(command: &HooksCommand) -> Result<(), String> {
    match command {
        HooksCommand::Plan { all, repo } => {
            let cfg = config::find_and_load()?;
            let targets: Vec<_> = if let Some(name) = repo {
                let r = cfg
                    .repos
                    .iter()
                    .find(|r| r.name == *name)
                    .ok_or_else(|| format!("Repo '{}' not found", name))?;
                vec![r]
            } else if *all {
                cfg.repos.iter().collect()
            } else {
                return Err("Use --all or --repo to specify repos for hook plan".to_string());
            };

            println!("Hook plan:");
            for repo in &targets {
                let repo_path = repo.resolved_path(&cfg);
                let hooks_dir = repo_path.join(".git").join("hooks");
                if !hooks_dir.exists() {
                    println!("  {}: no .git/hooks", repo.name);
                    continue;
                }
                let is_root = repo.name == "phenix";
                for hook_name in &["pre-commit", "pre-push"] {
                    let hook_path = hooks_dir.join(hook_name);
                    let status = if hook_path.exists() {
                        let existing = std::fs::read_to_string(&hook_path).unwrap_or_default();
                        if existing.contains("# managed-by: phenix-stitch-hooks") {
                            "managed (will overwrite)"
                        } else {
                            "unmanaged (will NOT overwrite unless --force)"
                        }
                    } else {
                        "absent (will create)"
                    };
                    println!("  {} {}: {}", repo.name, hook_name, status);
                    if is_root {
                        println!("    -> includes --affected-dag");
                    } else {
                        println!("    -> no --affected-dag (submodule-local)");
                    }
                }
            }
            Ok(())
        }
        HooksCommand::Install { all, repo, force } => {
            let cfg = config::find_and_load()?;
            let targets: Vec<_> = if let Some(name) = repo {
                vec![cfg
                    .repos
                    .iter()
                    .find(|r| r.name == *name)
                    .ok_or_else(|| format!("Repo '{}' not found", name))?]
            } else if *all {
                cfg.repos.iter().collect()
            } else {
                return Err("Use --all or --repo to install hooks".to_string());
            };

            let root = cfg.config_dir.as_deref().unwrap_or(Path::new("."));
            let mut installed = 0usize;

            for repo in &targets {
                let repo_path = repo.resolved_path(&cfg);
                match exec::install_hooks_for_repo(&repo.name, &repo_path, root, *force) {
                    Ok(result) if result.installed => {
                        installed += 1;
                        println!("Installed hooks for '{}'", repo.name);
                    }
                    Ok(result) => println!("Skipping {} ({})", repo.name, result.message),
                    Err(e) if e.contains("Not overwriting unmanaged") => {
                        eprintln!("WARNING: {e}");
                    }
                    Err(e) => return Err(e),
                }
            }

            if installed == 0 {
                println!("No repos with .git/hooks found.");
            } else {
                println!("\nInstalled hooks for {} repo(s).", installed);
            }
            Ok(())
        }
    }
}

fn validate_single_selection(
    all: bool,
    changed: bool,
    dirty: bool,
    node: Option<&str>,
    nodes: &[String],
    allow_empty: bool,
) -> Result<(), String> {
    let count = all as u32
        + changed as u32
        + dirty as u32
        + node.is_some() as u32
        + (!nodes.is_empty()) as u32;
    if count == 0 && !allow_empty {
        return Err("Must specify one of: --all, --changed, --dirty, --node, --nodes".to_string());
    }
    if count > 1 {
        return Err(
            "Must specify exactly one selection mode (--all, --changed, --dirty, --node, --nodes)"
                .to_string(),
        );
    }
    Ok(())
}

fn cmd_exec(
    all_flag: bool,
    changed: bool,
    dirty: bool,
    node: Option<&str>,
    nodes: &[String],
    closure: &str,
    order: &str,
    mode: &str,
    steps: &[String],
    dry_run: bool,
    apply: bool,
    json: bool,
    trailing_command: &[String],
) -> Result<(), String> {
    validate_single_selection(all_flag, changed, dirty, node, nodes, false)?;
    let cfg = config::find_and_load()?;

    let selection = if all_flag {
        exec::SelectionMode::All
    } else if changed {
        exec::SelectionMode::Changed
    } else if dirty {
        exec::SelectionMode::Dirty
    } else {
        exec::SelectionMode::Explicit
    };

    let explicit_nodes = if let Some(n) = node {
        vec![n.to_string()]
    } else {
        nodes.to_vec()
    };

    let closure_mode = exec::parse_closure_mode(closure)?;
    let order_mode = exec::parse_order_mode(order)?;
    let exec_mode = exec::parse_execution_mode(mode)?;

    let exec_steps = if steps.is_empty() && !trailing_command.is_empty() {
        vec![exec::ExecutionStep {
            id: "cmd".to_string(),
            mode: exec_mode,
            kind: exec::StepKind::Shell {
                argv: trailing_command.to_vec(),
            },
            condition: None,
        }]
    } else if !steps.is_empty() {
        steps
            .iter()
            .enumerate()
            .map(|(idx, s)| exec::ExecutionStep {
                id: format!("step-{}", idx + 1),
                mode: exec_mode,
                kind: exec::StepKind::Shell {
                    argv: vec!["sh".to_string(), "-c".to_string(), s.to_string()],
                },
                condition: None,
            })
            .collect()
    } else {
        return Err("Must provide --step or a trailing command".to_string());
    };

    let scope = exec::ExecutionScope {
        selection,
        explicit_nodes,
        closure: closure_mode,
        order: order_mode,
    };

    let plan = exec::build_plan(&cfg, &scope, exec_steps)?;

    if dry_run || json {
        exec::print_plan(&plan, json);
        if dry_run {
            return Ok(());
        }
    }

    let opts = exec::RunOptions {
        dry_run,
        apply,
        json,
    };
    let report = exec::run_plan(&cfg, &plan, &opts)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!(
            "Exec: {}/{} nodes successful, {} failed",
            report.successful_nodes, report.total_nodes, report.failed_nodes
        );
    }

    if report.failed_nodes > 0 {
        return Err("Some nodes failed".to_string());
    }
    Ok(())
}

fn cmd_verify(
    node: Option<&str>,
    nodes: &[String],
    all: bool,
    changed: bool,
    dirty: bool,
    upstream: bool,
    downstream: bool,
    dry_run: bool,
    json: bool,
) -> Result<(), String> {
    validate_single_selection(all, changed, dirty, node, nodes, true)?;
    let cfg = config::find_and_load()?;

    let selection = if all {
        exec::SelectionMode::All
    } else if changed {
        exec::SelectionMode::Changed
    } else if dirty {
        exec::SelectionMode::Dirty
    } else if node.is_some() || !nodes.is_empty() {
        exec::SelectionMode::Explicit
    } else {
        exec::SelectionMode::Changed
    };

    let explicit_nodes = if let Some(n) = node {
        vec![n.to_string()]
    } else {
        nodes.to_vec()
    };

    let closure = if upstream && downstream {
        exec::ClosureMode::Connected
    } else if upstream {
        exec::ClosureMode::Upstream
    } else {
        exec::ClosureMode::Downstream
    };

    let order = exec::OrderMode::ProvidersFirst;

    let steps = vec![exec::ExecutionStep {
        id: "tend-check".to_string(),
        mode: exec::ExecutionMode::ReadOnly,
        kind: exec::StepKind::Builtin {
            name: "tend.check".to_string(),
            args: serde_json::json!({"profile": "pre-push"}),
        },
        condition: None,
    }];

    let scope = exec::ExecutionScope {
        selection,
        explicit_nodes,
        closure,
        order,
    };

    let plan = exec::build_plan(&cfg, &scope, steps)?;

    if dry_run || json {
        exec::print_plan(&plan, json);
        if dry_run {
            return Ok(());
        }
    }

    let opts = exec::RunOptions {
        dry_run,
        apply: false,
        json,
    };
    let report = exec::run_plan(&cfg, &plan, &opts)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!(
            "Verify: {}/{} nodes passed, {} failed",
            report.successful_nodes, report.total_nodes, report.failed_nodes
        );
    }

    if report.failed_nodes > 0 {
        return Err("Some nodes failed verification".to_string());
    }
    Ok(())
}

fn cmd_recipe(command: &RecipeCommand) -> Result<(), String> {
    let cfg = config::find_and_load()?;
    let root = cfg.config_dir.as_deref().unwrap_or(Path::new("."));
    let collection = recipe::load_recipes(root)?;

    match command {
        RecipeCommand::List { json } => {
            recipe::list_recipes(&collection, *json);
            Ok(())
        }
        RecipeCommand::Plan {
            name,
            node,
            nodes,
            json,
        } => {
            let def = recipe::find_recipe(&collection, name)?;
            let resolved = recipe::resolve_recipe(def)?;
            let explicit_nodes = if let Some(n) = node {
                vec![n.clone()]
            } else {
                nodes.clone()
            };
            recipe::plan_recipe(&cfg, &resolved, &explicit_nodes, *json)
        }
        RecipeCommand::Run {
            name,
            node,
            nodes,
            dry_run,
            apply,
            json,
        } => {
            let def = recipe::find_recipe(&collection, name)?;
            let resolved = recipe::resolve_recipe(def)?;
            let explicit_nodes = if let Some(n) = node {
                vec![n.clone()]
            } else {
                nodes.clone()
            };
            let opts = exec::RunOptions {
                dry_run: *dry_run,
                apply: *apply,
                json: *json,
            };
            let report = recipe::run_recipe(&cfg, &resolved, &explicit_nodes, &opts)?;
            if report.failed_nodes > 0 {
                return Err(format!(
                    "Recipe '{}' completed with {} failed node(s)",
                    name, report.failed_nodes
                ));
            }
            Ok(())
        }
    }
}

fn cmd_changeset(command: &ChangesetCliCommand) -> Result<(), String> {
    match command {
        ChangesetCliCommand::New { title } => stitch::changeset::new::execute(title),
        ChangesetCliCommand::Status { json } => {
            let cs = stitch::changeset::load_current()?;
            match cs {
                Some(cs) => {
                    if *json {
                        let output = serde_json::to_string_pretty(&cs)
                            .map_err(|e| format!("JSON: {}", e))?;
                        println!("{}", output);
                    } else {
                        println!("Changeset: {} ({})", cs.id, cs.title);
                        println!("State: {}", cs.state);
                        println!("Workspace: {}", cs.workspace);
                        println!();
                        for rp in &cs.repos {
                            let action = rp.action.as_deref().unwrap_or("-");
                            let msg = rp.message.as_deref().unwrap_or("<missing>");
                            let hash = rp.commit_hash.as_deref().unwrap_or("-");
                            println!(
                                "  {}  action={}  message={}  hash={}",
                                rp.name, action, msg, hash
                            );
                        }
                    }
                }
                None => {
                    println!("No active changeset.");
                }
            }
            Ok(())
        }
        ChangesetCliCommand::Plan { write, json } => {
            stitch::changeset::plan::execute(*write, *json)
        }
        ChangesetCliCommand::SetMessage { repo, message } => {
            stitch::changeset::set_message::execute(repo, message)
        }
        ChangesetCliCommand::SetFiles { repo, files } => {
            stitch::changeset::set_files::execute(repo, files)
        }
        ChangesetCliCommand::Validate { json } => stitch::changeset::validate::execute(*json),
        ChangesetCliCommand::Commit => stitch::changeset::commit::execute(),
        ChangesetCliCommand::Push => stitch::changeset::push::execute(),
        ChangesetCliCommand::Abort => stitch::changeset::abort::execute(),
    }
}

#[derive(Parser)]
#[command(
    name = "stitch",
    version,
    about = "Multi-repo Git coordinator for Phenix workspaces"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List configured repos
    Repos,
    /// Show multi-repo status (like git status across all repos)
    Status {
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(long, help = "Short format")]
        short: bool,
        #[arg(long, help = "Dirty repos only")]
        dirty_only: bool,
        #[arg(long, help = "Filter by repo name")]
        repo: Option<String>,
    },
    /// Show diffs across repos
    Diff {
        #[arg(long, help = "Repo name")]
        repo: Option<String>,
        #[arg(long, help = "Show staged changes only")]
        staged: bool,
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    /// Show ordered operation DAG (read-only)
    Dag {
        #[arg(long, default_value = "commit", help = "DAG mode: commit, sync, full")]
        mode: Option<String>,
        #[arg(
            long,
            default_value = "by-repo",
            help = "Split strategy: by-repo, by-path"
        )]
        split: Option<String>,
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    /// Commit changed files in DAG dependency order (local commits only)
    Commit {
        #[arg(long, help = "Dry run (show plan, no mutations)")]
        dry_run: bool,
        #[arg(long, help = "JSON output for agent usage")]
        json: bool,
        #[arg(long, help = "Apply (required for actual commits)")]
        apply: bool,
        #[arg(long, help = "Allow edge cases like detached HEAD")]
        force: bool,
        #[arg(long, help = "Transaction ID to resume")]
        resume: Option<String>,
        #[arg(long, help = "Path to JSON messages file (from commit_template)")]
        messages: Option<String>,
        #[arg(long, help = "Write .stitch/messages.json and exit")]
        write_template: bool,
        #[arg(
            short = 'm',
            long = "message",
            help = "Commit message; only valid with --repo"
        )]
        message: Option<String>,
        #[arg(long, help = "Repo name")]
        repo: Option<String>,
    },
    /// Push committed changes in DAG dependency order
    Push {
        #[arg(long, help = "Dry run (show what would be pushed)")]
        dry_run: bool,
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    /// Sync/update/push in dependency order (update flake inputs, validate, push)
    Sync {
        #[arg(long, default_value = "push", help = "Mode: pull, push, full")]
        mode: Option<String>,
        #[arg(long, help = "Apply (required for actual sync operations)")]
        apply: bool,
        #[arg(long, help = "Dry run (show plan, no mutations)")]
        dry_run: bool,
        #[arg(long, help = "JSON output for agent usage")]
        json: bool,
        #[arg(long, help = "Skip push step")]
        no_push: bool,
        #[arg(long, help = "Filter by repo names")]
        repos: Vec<String>,
        #[arg(long, help = "Run tend checks before sync")]
        run_tend: bool,
    },
    /// Graph operations: derive, verify, order, print
    Graph {
        #[command(subcommand)]
        command: GraphCliCommand,
    },
    /// Topology operations: check, graph
    Topology {
        #[command(subcommand)]
        command: TopologyCommand,
    },
    /// Run arbitrary commands over selected DAG scopes
    Exec {
        #[arg(long, help = "Select all nodes")]
        all: bool,
        #[arg(long, help = "Select changed nodes")]
        changed: bool,
        #[arg(long, help = "Select dirty nodes")]
        dirty: bool,
        #[arg(long, help = "Select a single node by name")]
        node: Option<String>,
        #[arg(long, value_delimiter = ',', help = "Select multiple nodes by name")]
        nodes: Vec<String>,
        #[arg(
            long,
            default_value = "self",
            help = "Closure mode: self, upstream, downstream, connected, all"
        )]
        closure: String,
        #[arg(
            long,
            default_value = "stable",
            help = "Order mode: stable, providers-first, consumers-first"
        )]
        order: String,
        #[arg(
            long,
            default_value = "readonly",
            help = "Execution mode: readonly, mutating"
        )]
        mode: String,
        #[arg(long, help = "Step command (can be specified multiple times)")]
        step: Vec<String>,
        #[arg(long, help = "Dry run (show plan, no mutations)")]
        dry_run: bool,
        #[arg(long, help = "Apply (required for mutating mode)")]
        apply: bool,
        #[arg(long, help = "Output as JSON")]
        json: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        trailing_command: Vec<String>,
    },
    /// Verify workspace (default: changed nodes + downstream + providers-first)
    Verify {
        #[arg(long, help = "Select a single node by name")]
        node: Option<String>,
        #[arg(long, value_delimiter = ',', help = "Select multiple nodes by name")]
        nodes: Vec<String>,
        #[arg(long, help = "Select all nodes")]
        all: bool,
        #[arg(long, help = "Select changed nodes")]
        changed: bool,
        #[arg(long, help = "Select dirty nodes")]
        dirty: bool,
        #[arg(long, help = "Include upstream dependencies")]
        upstream: bool,
        #[arg(long, help = "Include downstream consumers")]
        downstream: bool,
        #[arg(long, help = "Dry run (show plan, no mutations)")]
        dry_run: bool,
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    /// Recipe operations: list, plan, run
    Recipe {
        #[command(subcommand)]
        command: RecipeCommand,
    },
    /// Install/plan Git hooks across workspace repos
    Hooks {
        #[command(subcommand)]
        command: HooksCommand,
    },
    /// Manage changesets (legacy, hidden)
    #[command(hide = true)]
    Changeset {
        #[command(subcommand)]
        command: ChangesetCliCommand,
    },
}

#[derive(Subcommand)]
enum RecipeCommand {
    /// List available recipes
    List {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    /// Show the execution plan for a recipe
    Plan {
        /// Recipe name
        name: String,
        #[arg(long, help = "Override selection with a specific node")]
        node: Option<String>,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Override selection with multiple nodes"
        )]
        nodes: Vec<String>,
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    /// Run a recipe
    Run {
        /// Recipe name
        name: String,
        #[arg(long, help = "Override selection with a specific node")]
        node: Option<String>,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Override selection with multiple nodes"
        )]
        nodes: Vec<String>,
        #[arg(long, help = "Dry run (show plan, no mutations)")]
        dry_run: bool,
        #[arg(long, help = "Apply (required for mutating recipes)")]
        apply: bool,
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
}

#[derive(Subcommand)]
enum GraphCliCommand {
    /// Derive workspace graph from lock files or metadata
    Derive {
        #[arg(long, default_value = ".", help = "Root workspace path")]
        workspace: String,
        #[arg(long, default_value = "locks", help = "Source: locks or json")]
        source: String,
        #[arg(long, help = "Path to workspace metadata file")]
        metadata: Option<String>,
        #[arg(
            long,
            default_value = "text",
            help = "Output format: text, json, mermaid"
        )]
        format: String,
    },
    /// Validate workspace graph topology
    Verify {
        #[arg(long, default_value = ".", help = "Root workspace path")]
        workspace: String,
        #[arg(long, default_value = "locks", help = "Source: locks or json")]
        source: String,
        #[arg(long, help = "Path to workspace metadata file")]
        metadata: Option<String>,
        #[arg(long, help = "Enable strict mode (warnings become errors)")]
        strict: bool,
        #[arg(long, default_value = "text", help = "Output format: text, json")]
        format: String,
    },
    /// Show provider-before-consumer order
    Order {
        #[arg(long, default_value = ".", help = "Root workspace path")]
        workspace: String,
        #[arg(long, default_value = "locks", help = "Source: locks or json")]
        source: String,
        #[arg(long, help = "Path to workspace metadata file")]
        metadata: Option<String>,
        #[arg(long, default_value = "text", help = "Output format: text, json")]
        format: String,
    },
    /// Print workspace graph
    Print {
        #[arg(long, default_value = ".", help = "Root workspace path")]
        workspace: String,
        #[arg(long, default_value = "locks", help = "Source: locks or json")]
        source: String,
        #[arg(long, help = "Path to workspace metadata file")]
        metadata: Option<String>,
        #[arg(
            long,
            default_value = "mermaid",
            help = "Output format: mermaid, json, text"
        )]
        format: String,
    },
}

#[derive(Subcommand)]
enum TopologyCommand {
    /// Validate workspace topology against the layer model
    Check {
        #[arg(long, default_value = ".", help = "Root workspace path")]
        workspace: String,
        #[arg(
            long,
            default_value = ".stitch/topology.json",
            help = "Path to topology config"
        )]
        config: String,
        #[arg(long, default_value = "text", help = "Output format: text, json")]
        format: String,
    },
    /// Render workspace topology as a graph
    Graph {
        #[arg(long, default_value = ".", help = "Root workspace path")]
        workspace: String,
        #[arg(
            long,
            default_value = ".stitch/topology.json",
            help = "Path to topology config"
        )]
        config: String,
        #[arg(
            long,
            default_value = "mermaid",
            help = "Output format: mermaid, json, text"
        )]
        format: String,
    },
}

#[derive(Subcommand)]
enum HooksCommand {
    /// Plan hook installation (show what would be installed)
    Plan {
        #[arg(long, help = "Plan for all repos")]
        all: bool,
        #[arg(long, help = "Plan for a specific repo")]
        repo: Option<String>,
    },
    /// Install hooks for workspace repos
    Install {
        #[arg(long, help = "Install hooks for all repos")]
        all: bool,
        #[arg(long, help = "Install hooks for a specific repo")]
        repo: Option<String>,
        #[arg(long, help = "Overwrite unmanaged hooks")]
        force: bool,
    },
}

#[derive(Subcommand)]
enum ChangesetCliCommand {
    /// Create a new changeset
    New {
        /// Title for the new changeset
        title: String,
    },
    /// Show current changeset status
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Build/review a plan for the current changeset
    Plan {
        /// Write the plan to the active changeset
        #[arg(long)]
        write: bool,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Set commit message for a repo in the changeset
    SetMessage {
        /// Repo name
        repo: String,
        /// Commit message
        message: String,
    },
    /// Set tracked files for a repo in the changeset
    SetFiles {
        /// Repo name
        repo: String,
        /// Files to track
        files: Vec<String>,
    },
    /// Validate the current changeset
    Validate {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Commit the validated changeset
    Commit,
    /// Push committed changeset repos
    Push,
    /// Abort the current changeset
    Abort,
}

fn cmd_repos() -> Result<(), String> {
    let cfg = config::find_and_load()?;
    for repo in &cfg.repos {
        let exists = if repo.resolved_path(&cfg).exists() {
            "\u{2713}"
        } else {
            "\u{2717}"
        };
        println!("{}  {}  ({})", exists, repo.name, repo.path);
    }
    Ok(())
}

fn cmd_status(
    json: bool,
    short: bool,
    dirty_only: bool,
    repo_filter: Option<&str>,
) -> Result<(), String> {
    let cfg = config::find_and_load()?;

    let explicit_nodes: Vec<String> = repo_filter.map(|r| vec![r.to_string()]).unwrap_or_default();
    let selection = if repo_filter.is_some() {
        exec::SelectionMode::Explicit
    } else {
        exec::SelectionMode::All
    };

    let scope = exec::ExecutionScope {
        selection,
        explicit_nodes,
        closure: exec::ClosureMode::SelfOnly,
        order: exec::OrderMode::Stable,
    };

    let steps = vec![exec::ExecutionStep {
        id: "collect-status".to_string(),
        mode: exec::ExecutionMode::ReadOnly,
        kind: exec::StepKind::Builtin {
            name: "git.collect-status".to_string(),
            args: serde_json::Value::Null,
        },
        condition: None,
    }];

    let plan = exec::build_plan(&cfg, &scope, steps)?;
    let opts = exec::RunOptions {
        dry_run: false,
        apply: false,
        json: false,
    };
    let report = exec::run_plan(&cfg, &plan, &opts)?;

    let mut statuses: Vec<serde_json::Value> = Vec::new();
    for nr in &report.node_results {
        for sr in &nr.step_results {
            if sr.success && !sr.stdout.is_empty() {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&sr.stdout) {
                    statuses.push(val);
                }
            }
        }
    }

    let filtered: Vec<&serde_json::Value> = statuses
        .iter()
        .filter(|s| !dirty_only || s.get("is_dirty").and_then(|v| v.as_bool()).unwrap_or(false))
        .collect();

    if json {
        let output = serde_json::to_string_pretty(&filtered).map_err(|e| format!("JSON: {}", e))?;
        println!("{}", output);
        return Ok(());
    }

    if short {
        for s in &filtered {
            let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let branch = s.get("branch").and_then(|v| v.as_str()).unwrap_or("?");
            let is_dirty = s.get("is_dirty").and_then(|v| v.as_bool()).unwrap_or(false);
            let prefix = if is_dirty { "M" } else { " " };
            println!("{}  {}  {}", prefix, name, branch);
        }
        return Ok(());
    }

    println!("Workspace: {}", cfg.workspace);
    println!();
    for s in &filtered {
        let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let branch = s.get("branch").and_then(|v| v.as_str()).unwrap_or("?");
        let is_dirty = s.get("is_dirty").and_then(|v| v.as_bool()).unwrap_or(false);
        let staged_count = s.get("staged_count").and_then(|v| v.as_u64()).unwrap_or(0);
        let unstaged_count = s
            .get("unstaged_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let untracked_count = s
            .get("untracked_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let is_present = s
            .get("is_present")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let dirty = if is_dirty { "yes" } else { "no" };
        println!("{}", name);
        println!("  branch: {}", branch);
        println!("  dirty: {}", dirty);
        println!("  staged: {}", staged_count);
        println!("  unstaged: {}", unstaged_count);
        println!("  untracked: {}", untracked_count);
        if is_dirty && is_present {
            let repo_cfg = cfg.repos.iter().find(|r| r.name == name);
            if let Some(r) = repo_cfg {
                let p = r.resolved_path(&cfg);
                let diff = git::git_diff_names(&p).unwrap_or_default();
                for f in &diff {
                    println!("    M {}", f);
                }
            }
        }
        if let Some(ahead) = s.get("ahead").and_then(|v| v.as_u64()) {
            if ahead > 0 {
                println!("  ahead: {}", ahead);
            }
        }
        println!();
    }
    Ok(())
}

fn cmd_diff(repo: Option<&str>, staged: bool, json: bool) -> Result<(), String> {
    let cfg = config::find_and_load()?;

    let explicit_nodes: Vec<String> = repo.map(|r| vec![r.to_string()]).unwrap_or_default();
    let selection = if repo.is_some() {
        exec::SelectionMode::Explicit
    } else {
        exec::SelectionMode::All
    };

    let scope = exec::ExecutionScope {
        selection,
        explicit_nodes,
        closure: exec::ClosureMode::SelfOnly,
        order: exec::OrderMode::Stable,
    };

    let steps = vec![exec::ExecutionStep {
        id: "git-diff".to_string(),
        mode: exec::ExecutionMode::ReadOnly,
        kind: exec::StepKind::Builtin {
            name: "git.diff".to_string(),
            args: serde_json::json!({ "staged": staged }),
        },
        condition: None,
    }];

    let plan = exec::build_plan(&cfg, &scope, steps)?;
    let opts = exec::RunOptions {
        dry_run: false,
        apply: false,
        json: false,
    };
    let report = exec::run_plan(&cfg, &plan, &opts)?;

    let mut all_diffs: Vec<serde_json::Value> = Vec::new();

    for nr in &report.node_results {
        for sr in &nr.step_results {
            if !sr.success {
                eprintln!("{} diff failed: {}", nr.node, sr.stderr);
                continue;
            }
            let diff_text = sr.stdout.trim().to_string();
            if diff_text.is_empty() {
                continue;
            }
            if json {
                all_diffs.push(serde_json::json!({
                    "repo": nr.node, "diff": diff_text
                }));
            } else {
                println!("--- {} ---", nr.node);
                println!("{}", diff_text);
            }
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "diffs": all_diffs
            }))
            .unwrap()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_selection_allows_empty_default() {
        assert!(validate_single_selection(false, false, false, None, &[], true).is_ok());
    }

    #[test]
    fn exec_selection_requires_selection() {
        assert!(validate_single_selection(false, false, false, None, &[], false).is_err());
    }

    #[test]
    fn selection_rejects_ambiguous_modes() {
        assert!(validate_single_selection(true, true, false, None, &[], true).is_err());
        assert!(validate_single_selection(
            false,
            false,
            false,
            Some("a"),
            &["b".to_string()],
            true
        )
        .is_err());
    }
}

fn cmd_dag(mode: Option<&str>, split: Option<&str>, json: bool) -> Result<(), String> {
    let mode = mode.unwrap_or("commit");
    let split = split.unwrap_or("by-repo");
    let cfg = config::find_and_load()?;
    let dag =
        stitch::graph::render::operation_dag_json(&cfg, mode, split, false, mode != "sync", &[])?;
    let nodes = dag
        .get("nodes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let evidence_message = dag.get("message").and_then(|v| v.as_str());

    if json {
        println!("{}", serde_json::to_string_pretty(&dag).unwrap());
    } else {
        let title = match mode {
            "sync" => "Sync DAG:",
            "full" => "Full DAG:",
            _ => "Commit DAG:",
        };
        println!("{}", title);
        if let Some(message) = evidence_message {
            println!("{}", message);
        }
        println!();
        for (i, n) in nodes.iter().enumerate() {
            let kind = n["kind"].as_str().unwrap_or("?");
            let id = n["id"].as_str().unwrap_or("?");
            println!("[{}] {}", i + 1, id);
            println!("    kind: {}", kind);
            if let Some(files) = n["files"].as_array() {
                if !files.is_empty() {
                    println!("    files:");
                    for f in files {
                        if let Some(p) = f.as_str() {
                            println!("      {}", p);
                        }
                    }
                }
            }
            if let Some(deps) = n["depends_on"].as_array() {
                if !deps.is_empty() {
                    println!("    depends_on:");
                    for d in deps {
                        if let Some(d) = d.as_str() {
                            println!("      {}", d);
                        }
                    }
                }
            }
            println!();
        }
        println!("Total: {} node(s)", nodes.len());
    }
    Ok(())
}

fn cmd_push(dry_run: bool, json_output: bool) -> Result<(), String> {
    let cfg = config::find_and_load()?;

    let scope = exec::ExecutionScope {
        selection: exec::SelectionMode::All,
        explicit_nodes: Vec::new(),
        closure: exec::ClosureMode::SelfOnly,
        order: exec::OrderMode::ProvidersFirst,
    };

    let steps = vec![exec::ExecutionStep {
        id: "git-push".to_string(),
        mode: exec::ExecutionMode::Mutating,
        kind: exec::StepKind::Builtin {
            name: "git.push".to_string(),
            args: serde_json::Value::Null,
        },
        condition: None,
    }];

    let plan = exec::build_plan(&cfg, &scope, steps)?;

    // Filter nodes with commits ahead of remote
    let mut to_push: Vec<(&exec::ExecutionNode, usize)> = Vec::new();
    for node in &plan.nodes {
        if !node.path.join(".git").exists() {
            continue;
        }
        let remote = git::git_remote(&node.path, "origin").ok();
        if remote.is_none() {
            continue;
        }
        let ahead = git::git_ahead_count(&node.path, "", "").unwrap_or(0);
        if ahead > 0 {
            to_push.push((node, ahead));
        }
    }

    if to_push.is_empty() {
        if json_output {
            println!(r#"{{"pushed": [], "message": "Nothing to push"}}"#);
        } else {
            println!("Nothing to push.");
        }
        return Ok(());
    }

    if dry_run {
        if json_output {
            let nodes: Vec<serde_json::Value> = to_push
                .iter()
                .map(|(n, ahead)| serde_json::json!({"name": n.name, "ahead": ahead}))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "push_order": to_push.iter().map(|(n, _)| n.name.clone()).collect::<Vec<_>>(),
                    "nodes": nodes
                }))
                .unwrap()
            );
        } else {
            println!("Would push (dependency order):");
            for (node, ahead) in &to_push {
                println!("  {} ({} ahead)", node.name, ahead);
            }
        }
        return Ok(());
    }

    // Build a filtered plan with only nodes that have commits to push
    let push_nodes: Vec<exec::ExecutionNode> = to_push.iter().map(|(n, _)| (*n).clone()).collect();
    let push_plan = exec::ExecutionPlan { nodes: push_nodes };

    let opts = exec::RunOptions {
        dry_run: false,
        apply: true,
        json: json_output,
    };

    if !json_output {
        println!("Pushing (dependency order):");
    }

    let report = exec::run_plan(&cfg, &push_plan, &opts)?;

    if json_output {
        let results: Vec<serde_json::Value> = report
            .node_results
            .iter()
            .map(|nr| {
                serde_json::json!({
                    "name": nr.node,
                    "success": nr.success,
                    "error": nr.step_results.first().map(|sr| sr.stderr.clone()).filter(|e| !e.is_empty())
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "pushed": results
            }))
            .unwrap()
        );
    }

    if report.failed_nodes > 0 {
        return Err("Some pushes failed".to_string());
    }

    Ok(())
}

fn load_commit_messages(path: &str) -> Result<BTreeMap<String, String>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("Read messages: {}", e))?;

    let raw_value: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Parse messages: {}", e))?;

    let raw_messages = raw_value
        .get("messages")
        .and_then(|v| v.as_object())
        .cloned()
        .or_else(|| raw_value.as_object().cloned())
        .ok_or_else(|| "messages file must be an object or {\"messages\": {...}}".to_string())?;

    let mut messages = BTreeMap::new();

    for (key, val) in raw_messages {
        let subject = val
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if subject.is_empty() {
            return Err(format!("Missing commit subject for '{}'", key));
        }

        messages.insert(key, subject.to_string());
    }

    Ok(messages)
}

fn cmd_commit(
    dry_run: bool,
    json_output: bool,
    apply: bool,
    _force: bool,
    resume_id: Option<&str>,
    messages_path: Option<&str>,
    write_template: bool,
    message: Option<String>,
    repo: Option<String>,
) -> Result<(), String> {
    let cfg = config::find_and_load()?;

    if write_template {
        let scope = exec::ExecutionScope {
            selection: exec::SelectionMode::All,
            explicit_nodes: Vec::new(),
            closure: exec::ClosureMode::SelfOnly,
            order: exec::OrderMode::Stable,
        };
        let steps = vec![exec::ExecutionStep {
            id: "collect-status".to_string(),
            mode: exec::ExecutionMode::ReadOnly,
            kind: exec::StepKind::Builtin {
                name: "git.collect-status".to_string(),
                args: serde_json::Value::Null,
            },
            condition: None,
        }];
        let plan = exec::build_plan(&cfg, &scope, steps)?;
        let opts = exec::RunOptions {
            dry_run: false,
            apply: false,
            json: false,
        };
        let report = exec::run_plan(&cfg, &plan, &opts)?;

        let mut template = serde_json::Map::new();
        for nr in &report.node_results {
            for sr in &nr.step_results {
                if sr.success && !sr.stdout.is_empty() {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&sr.stdout) {
                        let name = val.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let is_dirty = val
                            .get("is_dirty")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if !is_dirty {
                            continue;
                        }
                        let repo_cfg = cfg.repos.iter().find(|r| r.name == name);
                        let diff = repo_cfg
                            .map(|r| {
                                git::git_diff_names(&r.resolved_path(&cfg)).unwrap_or_default()
                            })
                            .unwrap_or_default();
                        template.insert(
                            name.to_string(),
                            serde_json::json!({ "subject": "", "body": "", "files": diff }),
                        );
                    }
                }
            }
        }
        let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {}", e))?;
        let msg_dir = cwd.join(".stitch");
        std::fs::create_dir_all(&msg_dir).map_err(|e| format!("Create .stitch dir: {}", e))?;
        let out_path = msg_dir.join("messages.json");
        let content = serde_json::to_string_pretty(&serde_json::json!({ "messages": template }))
            .map_err(|e| format!("Serialize: {}", e))?;
        std::fs::write(&out_path, &content)
            .map_err(|e| format!("Write {}: {}", out_path.display(), e))?;
        println!("Wrote {}", out_path.display());
        return Ok(());
    }

    if let Some(tx_id) = resume_id {
        let result = sync::resume_local_commit(tx_id, &cfg)?;
        println!("{}", sync::format_result_output(&result, json_output));
        return Ok(());
    }

    let explicit_nodes: Vec<String> = repo.clone().map(|r| vec![r]).unwrap_or_default();
    let selection = if repo.is_some() {
        exec::SelectionMode::Explicit
    } else {
        exec::SelectionMode::Changed
    };

    let scope = exec::ExecutionScope {
        selection,
        explicit_nodes: explicit_nodes.clone(),
        closure: exec::ClosureMode::Connected,
        order: exec::OrderMode::ProvidersFirst,
    };

    let raw_nodes = exec::build_scope(&cfg, &scope)?;
    let dirty_nodes: Vec<&exec::ExecutionNode> =
        raw_nodes.iter().filter(|n| n.directly_changed).collect();

    if dirty_nodes.is_empty() {
        if json_output {
            println!(r#"{{"commits": [], "message": "Nothing to commit"}}"#);
        } else {
            println!("Nothing to commit.");
        }
        return Ok(());
    }

    let _commit_names: std::collections::BTreeSet<String> =
        dirty_nodes.iter().map(|n| n.name.clone()).collect();

    let mut messages: Option<BTreeMap<String, String>> = if let Some(path) = messages_path {
        Some(load_commit_messages(path)?)
    } else {
        None
    };

    if let Some(r) = repo.as_ref() {
        if let Some(m) = message.clone() {
            let subject = m.trim();
            if subject.is_empty() {
                return Err("--message must not be empty".to_string());
            }
            let msgs = messages.get_or_insert_with(BTreeMap::new);
            msgs.insert(r.clone(), subject.to_string());
        }
    } else {
        if let Some(m) = message {
            if dirty_nodes.len() != 1 {
                return Err(
                    "--message requires --repo when more than one repo has changes".to_string(),
                );
            }
            let subject = m.trim();
            if subject.is_empty() {
                return Err("--message must not be empty".to_string());
            }
            let msgs = messages.get_or_insert_with(BTreeMap::new);
            msgs.insert(dirty_nodes[0].name.clone(), subject.to_string());
        }
    }

    // Build per-node commit steps with messages
    let mut commit_nodes: Vec<exec::ExecutionNode> = Vec::new();
    for node in dirty_nodes {
        let has_message = messages
            .as_ref()
            .and_then(|m| m.get(&node.name))
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        if !has_message && !dry_run {
            return Err(format!(
                "Missing explicit commit message for '{}'. Use --messages or --repo <name> -m <message>.",
                node.name
            ));
        }

        let msg = messages
            .as_ref()
            .and_then(|m| m.get(&node.name))
            .cloned()
            .unwrap_or_default();

        let mut n = node.clone();
        n.steps = vec![exec::ExecutionStep {
            id: "git-commit".to_string(),
            mode: exec::ExecutionMode::Mutating,
            kind: exec::StepKind::Builtin {
                name: "git.commit".to_string(),
                args: serde_json::json!({"message": msg, "stage": true}),
            },
            condition: None,
        }];
        commit_nodes.push(n);
    }

    if commit_nodes.is_empty() {
        if json_output {
            println!(r#"{{"commits": [], "message": "Nothing to commit"}}"#);
        } else {
            println!("Nothing to commit.");
        }
        return Ok(());
    }

    let plan = exec::ExecutionPlan {
        nodes: commit_nodes,
    };

    if dry_run {
        exec::print_plan(&plan, json_output);
        return Ok(());
    }

    if !apply {
        exec::print_plan(&plan, json_output);
        return Err("Set --apply to execute the commit, or use --dry-run to preview.".to_string());
    }

    let opts = exec::RunOptions {
        dry_run: false,
        apply: true,
        json: json_output,
    };

    let report = exec::run_plan(&cfg, &plan, &opts)?;

    if json_output {
        let results: Vec<serde_json::Value> = report
            .node_results
            .iter()
            .map(|nr| {
                serde_json::json!({
                    "name": nr.node,
                    "success": nr.success,
                    "error": nr.step_results.first().map(|sr| sr.stderr.clone()).filter(|e| !e.is_empty())
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "committed": results
            }))
            .unwrap()
        );
    }

    if report.failed_nodes > 0 {
        return Err("Some commits failed".to_string());
    }

    Ok(())
}

fn cmd_sync(
    mode: Option<&str>,
    apply: bool,
    dry_run: bool,
    json_output: bool,
    no_push: bool,
    repos: &[String],
    run_tend: bool,
) -> Result<(), String> {
    let mode = mode.unwrap_or("push");
    let (update_inputs, push_outputs) = match mode {
        "pull" => (true, false),
        "push" => (false, true),
        "full" => (true, true),
        other => {
            return Err(format!(
                "Unknown sync mode '{other}' (use: pull, push, full)"
            ))
        }
    };
    let cfg = config::find_and_load()?;

    let explicit_nodes: Vec<String> = if repos.is_empty() {
        Vec::new()
    } else {
        for name in repos {
            if !cfg.repos.iter().any(|r| r.name == *name) {
                return Err(format!("Unknown repo '{name}'"));
            }
        }
        repos.to_vec()
    };

    let selection = if !explicit_nodes.is_empty() {
        exec::SelectionMode::Explicit
    } else {
        exec::SelectionMode::Changed
    };

    let scope = exec::ExecutionScope {
        selection,
        explicit_nodes,
        closure: exec::ClosureMode::Connected,
        order: exec::OrderMode::ProvidersFirst,
    };

    let raw_nodes = exec::build_scope(&cfg, &scope)?;
    let dirty_nodes: Vec<&exec::ExecutionNode> = raw_nodes
        .iter()
        .filter(|n| n.directly_changed || n.downstream_only)
        .collect();

    if dirty_nodes.is_empty() {
        if json_output {
            println!(r#"{{"synced": [], "message": "Nothing to sync"}}"#);
        } else {
            println!("Nothing to sync.");
        }
        return Ok(());
    }

    // Build per-node sync steps: update inputs + tend check + push
    let mut sync_nodes: Vec<exec::ExecutionNode> = Vec::new();
    for node in dirty_nodes {
        let mut steps: Vec<exec::ExecutionStep> = Vec::new();

        // Update flake lock inputs (only for nodes with lockfiles)
        if update_inputs && node.path.join("flake.lock").exists() {
            steps.push(exec::ExecutionStep {
                id: "update-inputs".to_string(),
                mode: exec::ExecutionMode::Mutating,
                kind: exec::StepKind::Builtin {
                    name: "nix.updateInputs".to_string(),
                    args: serde_json::Value::Null,
                },
                condition: Some(exec::StepCondition::HasLockfile),
            });
        }

        // Run tend checks
        if run_tend {
            steps.push(exec::ExecutionStep {
                id: "tend-check".to_string(),
                mode: exec::ExecutionMode::ReadOnly,
                kind: exec::StepKind::Builtin {
                    name: "tend.check".to_string(),
                    args: serde_json::json!({"profile": "pre-push", "affected_dag": true}),
                },
                condition: Some(exec::StepCondition::DirectlyChanged),
            });
        }

        // Push (unless --no-push)
        if push_outputs && !no_push {
            steps.push(exec::ExecutionStep {
                id: "git-push".to_string(),
                mode: exec::ExecutionMode::Mutating,
                kind: exec::StepKind::Builtin {
                    name: "git.push".to_string(),
                    args: serde_json::Value::Null,
                },
                condition: Some(exec::StepCondition::DirectlyChanged),
            });
        }

        if !steps.is_empty() {
            let mut n = node.clone();
            n.steps = steps;
            sync_nodes.push(n);
        }
    }

    if sync_nodes.is_empty() {
        if json_output {
            println!(r#"{{"synced": [], "message": "Nothing to sync"}}"#);
        } else {
            println!("Nothing to sync.");
        }
        return Ok(());
    }

    let plan = exec::ExecutionPlan { nodes: sync_nodes };

    if dry_run {
        exec::print_plan(&plan, json_output);
        return Ok(());
    }

    if !apply {
        exec::print_plan(&plan, json_output);
        return Err("Set --apply to execute the sync, or use --dry-run to preview.".to_string());
    }

    let opts = exec::RunOptions {
        dry_run: false,
        apply: true,
        json: json_output,
    };

    let report = exec::run_plan(&cfg, &plan, &opts)?;

    if json_output {
        let results: Vec<serde_json::Value> = report
            .node_results
            .iter()
            .map(|nr| {
                serde_json::json!({
                    "name": nr.node,
                    "success": nr.success,
                    "error": nr.step_results.first().map(|sr| sr.stderr.clone()).filter(|e| !e.is_empty())
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "synced": results
            }))
            .unwrap()
        );
    }

    if report.failed_nodes > 0 {
        return Err("Some sync operations failed".to_string());
    }

    Ok(())
}
