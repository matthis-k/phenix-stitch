use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::exec::{
    self, build_plan, parse_closure_mode, parse_execution_mode, parse_order_mode,
    parse_selection_mode, ClosureMode, ExecutionMode, ExecutionScope, ExecutionStep, OrderMode,
    RunOptions, SelectionMode, StepKind,
};

const KNOWN_BUILTINS: &[&str] = &[
    "git.status",
    "git.collect-status",
    "git.diff",
    "git.commit",
    "git.push",
    "tend.check",
    "nix.updateInputs",
    "hooks.install",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeCollection {
    pub version: u32,
    pub recipes: Vec<RecipeDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeDef {
    pub name: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default = "default_selection")]
    pub selection: String,
    #[serde(default = "default_closure")]
    pub closure: String,
    #[serde(default = "default_order")]
    pub order: String,
    pub steps: Vec<RecipeStepDef>,
}

fn default_mode() -> String {
    "readonly".to_string()
}

fn default_selection() -> String {
    "all".to_string()
}

fn default_closure() -> String {
    "self".to_string()
}

fn default_order() -> String {
    "stable".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeStepDef {
    pub id: String,
    #[serde(default)]
    pub run: Option<Vec<String>>,
    #[serde(default)]
    pub builtin: Option<String>,
    #[serde(default)]
    pub if_field: Option<String>,
    #[serde(default)]
    pub args: Option<serde_json::Value>,
}

pub fn load_recipes(root: &Path) -> Result<RecipeCollection, String> {
    let recipes_path = root.join(".stitch").join("recipes.json");
    if !recipes_path.exists() {
        return Ok(RecipeCollection {
            version: 1,
            recipes: Vec::new(),
        });
    }
    let content = std::fs::read_to_string(&recipes_path)
        .map_err(|e| format!("Failed to read {}: {}", recipes_path.display(), e))?;
    let collection: RecipeCollection = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", recipes_path.display(), e))?;
    if collection.version < 1 {
        return Err(format!("Unsupported recipe version {}", collection.version));
    }
    Ok(collection)
}

pub fn find_recipe<'a>(
    collection: &'a RecipeCollection,
    name: &str,
) -> Result<&'a RecipeDef, String> {
    collection
        .recipes
        .iter()
        .find(|r| r.name == name)
        .ok_or_else(|| format!("Recipe '{name}' not found"))
}

pub fn resolve_recipe(recipe: &RecipeDef) -> Result<RecipeResolved, String> {
    let selection = parse_selection_mode(&recipe.selection)?;
    let closure = parse_closure_mode(&recipe.closure)?;
    let order = parse_order_mode(&recipe.order)?;
    let mode = parse_execution_mode(&recipe.mode)?;

    let mut steps = Vec::new();
    for step_def in &recipe.steps {
        let step_kind = if let Some(ref run) = step_def.run {
            if run.is_empty() {
                return Err(format!("Step '{}': empty run command", step_def.id));
            }
            StepKind::Shell { argv: run.clone() }
        } else if let Some(ref builtin) = step_def.builtin {
            if !KNOWN_BUILTINS.contains(&builtin.as_str()) {
                return Err(format!(
                    "Step '{}': unknown built-in '{}'. Known built-ins: {}",
                    step_def.id,
                    builtin,
                    KNOWN_BUILTINS.join(", ")
                ));
            }
            let args = step_def.args.clone().unwrap_or(serde_json::Value::Null);
            StepKind::Builtin {
                name: builtin.clone(),
                args,
            }
        } else {
            return Err(format!(
                "Step '{}': must have either 'run' or 'builtin'",
                step_def.id
            ));
        };

        let condition = if let Some(ref if_str) = step_def.if_field {
            Some(exec::parse_condition(if_str)?)
        } else {
            None
        };

        steps.push(ExecutionStep {
            id: step_def.id.clone(),
            mode,
            kind: step_kind,
            condition,
        });
    }

    Ok(RecipeResolved {
        name: recipe.name.clone(),
        selection,
        closure,
        order,
        mode,
        steps,
    })
}

#[derive(Debug, Clone)]
pub struct RecipeResolved {
    pub name: String,
    pub selection: SelectionMode,
    pub closure: ClosureMode,
    pub order: OrderMode,
    pub mode: ExecutionMode,
    pub steps: Vec<ExecutionStep>,
}

pub fn list_recipes(collection: &RecipeCollection, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(&collection).unwrap());
    } else {
        if collection.recipes.is_empty() {
            println!("No recipes found in .stitch/recipes.json");
            return;
        }
        println!("Available recipes:");
        for recipe in &collection.recipes {
            println!(
                "  {}  (mode: {}, selection: {}, closure: {}, order: {}, steps: {})",
                recipe.name,
                recipe.mode,
                recipe.selection,
                recipe.closure,
                recipe.order,
                recipe.steps.len()
            );
        }
    }
}

pub fn plan_recipe(
    cfg: &crate::model::WorkspaceConfig,
    resolved: &RecipeResolved,
    explicit_nodes: &[String],
    json: bool,
) -> Result<(), String> {
    let scope = ExecutionScope {
        selection: resolved.selection,
        explicit_nodes: explicit_nodes.to_vec(),
        closure: resolved.closure,
        order: resolved.order,
    };

    let plan = build_plan(cfg, &scope, resolved.steps.clone())?;
    exec::print_plan(&plan, json);
    Ok(())
}

pub fn run_recipe(
    cfg: &crate::model::WorkspaceConfig,
    resolved: &RecipeResolved,
    explicit_nodes: &[String],
    opts: &RunOptions,
) -> Result<exec::ExecutionReport, String> {
    let scope = ExecutionScope {
        selection: resolved.selection,
        explicit_nodes: explicit_nodes.to_vec(),
        closure: resolved.closure,
        order: resolved.order,
    };

    let plan = build_plan(cfg, &scope, resolved.steps.clone())?;

    if opts.dry_run || opts.json {
        exec::print_plan(&plan, opts.json);
        if opts.dry_run {
            return Ok(exec::ExecutionReport {
                node_results: Vec::new(),
                total_nodes: 0,
                successful_nodes: 0,
                failed_nodes: 0,
            });
        }
    }

    let report = exec::run_plan(cfg, &plan, opts)?;

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        println!(
            "Recipe '{}' completed: {}/{} nodes successful, {} failed",
            resolved.name, report.successful_nodes, report.total_nodes, report.failed_nodes
        );
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_recipe_status() {
        let recipe = RecipeDef {
            name: "status".to_string(),
            mode: "readonly".to_string(),
            selection: "all".to_string(),
            closure: "self".to_string(),
            order: "stable".to_string(),
            steps: vec![RecipeStepDef {
                id: "git-status".to_string(),
                run: Some(vec![
                    "git".to_string(),
                    "status".to_string(),
                    "--short".to_string(),
                ]),
                builtin: None,
                if_field: None,
                args: None,
            }],
        };
        let resolved = resolve_recipe(&recipe).unwrap();
        assert_eq!(resolved.name, "status");
        assert_eq!(resolved.mode, ExecutionMode::ReadOnly);
        assert_eq!(resolved.steps.len(), 1);
    }

    #[test]
    fn test_resolve_recipe_invalid_closure() {
        let recipe = RecipeDef {
            name: "bad".to_string(),
            mode: "readonly".to_string(),
            selection: "all".to_string(),
            closure: "foo".to_string(),
            order: "stable".to_string(),
            steps: vec![],
        };
        assert!(resolve_recipe(&recipe).is_err());
    }

    #[test]
    fn test_resolve_recipe_invalid_order() {
        let recipe = RecipeDef {
            name: "bad".to_string(),
            mode: "readonly".to_string(),
            selection: "all".to_string(),
            closure: "self".to_string(),
            order: "foo".to_string(),
            steps: vec![],
        };
        assert!(resolve_recipe(&recipe).is_err());
    }

    #[test]
    fn test_resolve_recipe_unknown_builtin_fails() {
        let recipe = RecipeDef {
            name: "bad-builtin".to_string(),
            mode: "readonly".to_string(),
            selection: "all".to_string(),
            closure: "self".to_string(),
            order: "stable".to_string(),
            steps: vec![RecipeStepDef {
                id: "step1".to_string(),
                run: None,
                builtin: Some("unknown.builtin".to_string()),
                if_field: None,
                args: None,
            }],
        };
        let result = resolve_recipe(&recipe);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown built-in") || err.contains("unknown.builtin"));
    }

    #[test]
    fn test_resolve_recipe_invalid_condition() {
        let recipe = RecipeDef {
            name: "bad-cond".to_string(),
            mode: "readonly".to_string(),
            selection: "all".to_string(),
            closure: "self".to_string(),
            order: "stable".to_string(),
            steps: vec![RecipeStepDef {
                id: "step1".to_string(),
                run: Some(vec!["echo".to_string()]),
                builtin: None,
                if_field: Some("invalid_condition".to_string()),
                args: None,
            }],
        };
        assert!(resolve_recipe(&recipe).is_err());
    }
}
