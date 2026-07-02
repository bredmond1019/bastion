use anyhow::Result;

pub mod youtube;

pub async fn run_native(workflow: String, args: Option<String>) -> Result<()> {
    println!("Native engine initialized. (Hybrid MCP Architecture)");
    println!("Workflow requested: {}", workflow);

    match workflow.as_str() {
        "youtube" => {
            let url = args.unwrap_or_default();
            youtube::run(url).await?;
        }
        _ => println!("Unknown workflow: {}", workflow),
    }

    Ok(())
}
