use anyhow::Result;
use cargo_metadata::{Metadata, MetadataCommand, Package};
use serde_derive::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::{
    fmt::Debug,
    fs, io,
    process::{Command, ExitStatus},
};

const SOROBAN_EXAMPLES_NAME: &str = "soroban-examples";
const SOROBAN_EXAMPLES_URL: &str = "https://github.com/stellar/soroban-examples.git";

static CONTRACT_LIST: &str = include_str!("../contract-list.toml");

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
struct ContractList {
    contracts: Vec<String>,
}

fn main() -> Result<()> {
    let current_dir = std::env::current_dir()?;
    let work_dir = current_dir.join("repro-test");
    std::fs::create_dir_all(&work_dir)?;

    clone_repo(SOROBAN_EXAMPLES_URL, &work_dir)?;

    let contract_list: ContractList = toml::from_str(CONTRACT_LIST)?;

    let project_dir = work_dir.join(&SOROBAN_EXAMPLES_NAME);

    // todo: remove this
    let project_dir = Path::new("../soroban-examples");

    for contract in contract_list.contracts {
        println!("------- contract: {contract} ----------");
        let contract_dir = project_dir.join(contract);
        let contract_manifest_path = contract_dir.join("Cargo.toml");
        if !contract_manifest_path.exists() {
            todo!();
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

            packages.iter().for_each(|p| {
                test_package(p, &work_dir).expect("Failed running test_package");
            });
        }
    }

    Ok(())
}

fn test_package(p: &Package, work_dir: &PathBuf) -> Result<()> {
    // print each step 

    let wasm_path = build_package(&p, &work_dir)?;
    let wasm_opt_path = wasm_opt(&wasm_path)?;
    wasm_repro(&wasm_path)?;
    wasm_repro(&wasm_opt_path)?;

    Ok(())
}

fn build_package(p: &Package, out_dir: &PathBuf) -> Result<PathBuf> {
    let soroban_path = std::env::current_exe().unwrap();
    let mut soroban_cmd = Command::new(&soroban_path);

    soroban_cmd.current_dir(PathBuf::from("../soroban-cli"));
    soroban_cmd.args([
        "contract",
        "build",
        "--manifest-path",
        &p.manifest_path.to_string(),
        "--package",
        &p.name,
        "--out-dir",
        &out_dir.to_string_lossy(),
    ]);
    println!("soroban_cmd: {:?}", soroban_cmd);
    let status = soroban_cmd.status()?;
    
    let file_name = format!("{}.wasm", p.name.replace('-', "_"));

    Ok(out_dir.join(file_name))
}

fn wasm_opt(p: &PathBuf) -> Result<PathBuf> {
    todo!()
}

fn wasm_repro(p: &PathBuf) -> Result<()> {
    Ok(())
}

fn clone_repo(git_url: &str, work_dir: &PathBuf) -> Result<()> {
    // todo
    Ok(())
}
