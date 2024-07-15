use anyhow::{anyhow, Result};
use cargo_metadata::{MetadataCommand, Package};
use clap::Parser;
use colored::*;
use petgraph::{algo, graph::DiGraph};
use rand::prelude::SliceRandom;
use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::{fmt::Debug, fs, process::Command};

const SOROBAN_EXAMPLES_URL: &str = "https://github.com/stellar/soroban-examples.git";
const SOROBAN_EXAMPLES_NAME: &str = "soroban-examples";

static CONTRACT_LIST: &str = include_str!("../contract-list.toml");

#[derive(Parser, Debug, Clone)]
pub enum Cmd {
    Build(BuildCmd),
    Repro(ReproCmd),
}

#[derive(Parser, Debug, Clone)]
pub struct BuildCmd {
    /// Path to the source code.
    #[arg(long)]
    project: Option<PathBuf>,
}

#[derive(Parser, Debug, Clone)]
pub struct ReproCmd {
    /// URL to the wasm files.
    #[arg(long)]
    wasm: PathBuf,
    /// Path to the source code.
    #[arg(long)]
    project: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct ContractList {
    contracts: Vec<String>,
}

fn main() -> Result<()> {
    let current_dir = std::env::current_dir()?;
    let work_dir = current_dir.join("repro-test");

    std::fs::create_dir_all(&work_dir)?;

    let cmd = Cmd::parse();
    match cmd {
        Cmd::Build(cmd) => cmd.run(&work_dir)?,
        Cmd::Repro(cmd) => cmd.run(&work_dir)?,
    }

    Ok(())
}

impl BuildCmd {
    pub fn run(&self, work_dir: &PathBuf) -> Result<()> {
        let wasm_out = work_dir.join("wasm-output");

        let mut project_dir = PathBuf::new();

        if let Some(dir) = &self.project {
            project_dir = dir.to_path_buf();
        } else {
            project_dir = work_dir.join(SOROBAN_EXAMPLES_NAME);
            clone_repo(SOROBAN_EXAMPLES_URL, &project_dir)?;
        }

        let contract_list: ContractList = toml::from_str(CONTRACT_LIST)?;
        for contract in contract_list.contracts {
            let contract_dir = project_dir.join(&contract);
            let contract_manifest_path = contract_dir.join("Cargo.toml");
            if !contract_manifest_path.exists() {
                return Err(anyhow!(
                    "Can't find manifest-path for contract {}",
                    contract
                ));
            } else {
                let metadata = MetadataCommand::new()
                    .manifest_path(contract_manifest_path)
                    .no_deps()
                    .exec()?;

                let packages: Vec<Package> = metadata
                    .packages
                    .iter()
                    .filter(|p| {
                        p.targets
                            .iter()
                            .any(|t| t.crate_types.iter().any(|c| c == "cdylib"))
                    })
                    .cloned()
                    .collect();

                for p in packages {
                    println!(
                        "{}",
                        format!(
                            "#### Building contract: {}, package: {} ####",
                            contract, p.name
                        )
                        .green()
                        .bold()
                    );
                    let wasm_file_path = build_package(&p, &wasm_out)?;

                    println!(
                        "{}",
                        format!("#### Optimizing file: {} ####", &wasm_file_path.display())
                            .green()
                            .bold()
                    );
                    wasm_opt(&wasm_file_path)?;
                }
            }
        }
        Ok(())
    }
}

impl ReproCmd {
    pub fn run(&self, work_dir: &PathBuf) -> Result<()> {
        let wasm_path = self.wasm.canonicalize()?;
        let wasm_files = find_wasm_files(&wasm_path)?;

        if wasm_files.is_empty() {
            return Err(anyhow!("Can't find wasm files under directory {}. Please provide the right wasm directory.", self.wasm.display()));
        }

        let mut project_dir = PathBuf::new();

        if let Some(dir) = &self.project {
            project_dir = dir.to_path_buf();
        } else {
            project_dir = work_dir.join(SOROBAN_EXAMPLES_NAME);
            clone_repo(SOROBAN_EXAMPLES_URL, &project_dir)?;
        }

        project_dir = project_dir.canonicalize()?;

        for wasm in wasm_files {
            println!(
                "{}",
                format!("#### Reproducing: {} ####", &wasm.display())
                    .green()
                    .bold()
            );
            wasm_repro(&wasm, &project_dir)?;
        }

        Ok(())
    }
}

fn build_package(p: &Package, out_dir: &PathBuf) -> Result<PathBuf> {
    let mut soroban_cmd = Command::new("cargo");
    soroban_cmd.current_dir(PathBuf::from("../soroban-cli"));

    let rustc = get_random_rustc();
    let rustc = "1.78.0"; // testing
    println!("Using rustc {}", &rustc);

    soroban_cmd.env("RUSTUP_TOOLCHAIN", &rustc);
    soroban_cmd.args([
        "run",
        "contract",
        "build",
        "--manifest-path",
        &p.manifest_path.to_string(),
        "--package",
        &p.name,
        "--out-dir",
        &out_dir.to_string_lossy(),
    ]);

    let status = soroban_cmd.status()?;

    if !status.success() {
        return Err(anyhow!("Failed running soroban contract build: {}", status));
    }
    let file_name = format!("{}.wasm", p.name.replace('-', "_"));

    let out_file_path = out_dir.join(file_name);
    Ok(out_file_path)
}

fn wasm_opt(wasm: &PathBuf) -> Result<PathBuf> {
    let mut wasm_out = PathBuf::from(wasm);
    wasm_out.set_extension("optimized.wasm");

    let mut soroban_cmd = Command::new("cargo");
    soroban_cmd.current_dir(PathBuf::from("../soroban-cli"));
    soroban_cmd.args([
        "run",
        "--features",
        "opt",
        "contract",
        "optimize",
        "--wasm",
        &wasm.to_string_lossy(),
        "--wasm-out",
        &wasm_out.to_string_lossy(),
    ]);

    let status = soroban_cmd.status()?;

    if !status.success() {
        return Err(anyhow!(
            "Failed running soroban contract optimize: {}",
            status
        ));
    }

    Ok(wasm_out)
}

fn wasm_repro(wasm: &PathBuf, project_dir: &PathBuf) -> Result<()> {
    let mut soroban_cmd = Command::new("cargo");
    soroban_cmd.current_dir(PathBuf::from("../soroban-cli"));
    soroban_cmd.args([
        "run",
        "--features",
        "opt",
        "contract",
        "repro",
        "--wasm",
        &wasm.to_string_lossy(),
        "--repo",
        &project_dir.to_string_lossy(),
    ]);

    let status = soroban_cmd.status()?;

    if !status.success() {
        return Err(anyhow!("Failed running soroban contract repro: {}", status));
    }

    Ok(())
}

fn find_wasm_files(wasm_dir: &PathBuf) -> Result<Vec<PathBuf>> {
    let mut path_vec = Vec::<PathBuf>::new();

    if !wasm_dir.is_dir() {
        return Err(anyhow!("Please provide the right wasm directory."));
    } else {
        for file in fs::read_dir(wasm_dir)? {
            let file = file?;
            let file_path = file.path();
            if file_path
                .extension()
                .is_some_and(|file| file.to_string_lossy().eq("wasm"))
            {
                path_vec.push(file_path.clone());
            }

            if file_path.is_dir() {
                find_wasm_files(&file_path)?;
            }
        }
    }

    path_vec = toposort(&path_vec)?;

    Ok(path_vec)
}

fn toposort(path_vec: &Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut soroban_cross_contract_a_contract = None;
    let mut soroban_cross_contract_b_contract = None;
    let mut soroban_atomic_swap_contract = None;
    let mut soroban_atomic_multiswap_contract = None;
    let mut soroban_token_contract = None;
    let mut soroban_liquidity_pool_contract = None;

    let mut deps = DiGraph::<PathBuf, ()>::new();
    let mut node_indices = HashMap::new();

    for path in path_vec {
        let path_str = path.to_string_lossy();
        let node_index = deps.add_node(path.clone());
        node_indices.insert(path.clone(), node_index);

        if path_str.contains("soroban_cross_contract_a_contract") {
            soroban_cross_contract_a_contract = Some(node_index);
        } else if path_str.contains("soroban_cross_contract_b_contract") {
            soroban_cross_contract_b_contract = Some(node_index);
        } else if path_str.contains("soroban_atomic_swap_contract") {
            soroban_atomic_swap_contract = Some(node_index);
        } else if path_str.contains("soroban_atomic_multiswap_contract") {
            soroban_atomic_multiswap_contract = Some(node_index);
        } else if path_str.contains("soroban_token_contract") {
            soroban_token_contract = Some(node_index);
        } else if path_str.contains("soroban_liquidity_pool_contract") {
            soroban_liquidity_pool_contract = Some(node_index);
        }
    }

    if let (Some(a), Some(b)) = (
        soroban_cross_contract_a_contract,
        soroban_cross_contract_b_contract,
    ) {
        deps.add_edge(a, b, ());
    }
    if let (Some(a), Some(b)) = (
        soroban_atomic_swap_contract,
        soroban_atomic_multiswap_contract,
    ) {
        deps.add_edge(a, b, ());
    }
    if let (Some(a), Some(b)) = (soroban_token_contract, soroban_liquidity_pool_contract) {
        deps.add_edge(a, b, ());
    }

    let sorted = algo::toposort(&deps, None).expect("Failed running toposort.");
    let sorted_paths = sorted.into_iter().map(|node| deps[node].clone()).collect();

    Ok(sorted_paths)
}

fn clone_repo(git_url: &str, work_dir: &PathBuf) -> Result<()> {
    let mut git_cmd = Command::new("git");
    git_cmd.args(["clone", git_url, &work_dir.to_string_lossy()]);
    git_cmd.status()?;

    Ok(())
}

fn get_random_rustc() -> String {
    let rustc_choices = ["1.79.0", "1.78.0", "1.77.2"];
    let mut rng = rand::thread_rng();

    let rustc = rustc_choices.choose(&mut rng).unwrap_or(&"1.79.0");

    rustc.to_string()
}
