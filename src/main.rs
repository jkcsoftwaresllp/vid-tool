use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio;

#[derive(Debug, Serialize, Deserialize)]
struct VideoData {
    assets: Vec<String>,
}

#[derive(Debug)]
struct VideoAutomation {
    template_path: PathBuf,
    assets_directory: PathBuf,
    output_directory: PathBuf,
    ae_path: PathBuf,
}

impl VideoAutomation {
    pub fn new<P: AsRef<Path>>(
        template_path: P,
        assets_directory: P,
        output_directory: P,
        ae_path: P,
    ) -> Self {
        VideoAutomation {
            template_path: template_path.as_ref().to_path_buf(),
            assets_directory: assets_directory.as_ref().to_path_buf(),
            output_directory: output_directory.as_ref().to_path_buf(),
            ae_path: ae_path.as_ref().to_path_buf(),
        }
    }

    pub async fn load_data<P: AsRef<Path>>(&self, json_path: P) -> Result<VideoData> {
        let content = tokio::fs::read_to_string(json_path)
            .await
            .context("Failed to read JSON file")?;
        let data: VideoData = serde_json::from_str(&content).context("Failed to parse JSON")?;
        Ok(data)
    }

    pub async fn get_assets(&self, asset_names: &[String]) -> Result<Vec<PathBuf>> {
        let mut assets = Vec::new();
        for asset_name in asset_names {
            let asset_path = self.assets_directory.join(format!("{}.png", asset_name));
            if asset_path.exists() {
                assets.push(asset_path);
            } else {
                anyhow::bail!("Asset not found: {}", asset_path.display());
            }
        }
        Ok(assets)
    }

    pub async fn prepare_project(&self, assets: &[PathBuf]) -> Result<PathBuf> {
        let temp_dir = self.output_directory.join("/");
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .context("Failed to create temp directory")?;

        let project_path = temp_dir.join("Teen Patti Video 2.aep");
        tokio::fs::copy(&self.template_path, &project_path)
            .await
            .context("Failed to copy template")?;

        // TODO: Modify the AEP file

        Ok(project_path)
    }

    pub async fn render_video<P: AsRef<Path>>(
        &self,
        project_path: P,
        output_name: &str,
    ) -> Result<PathBuf> {
        let output_path = self.output_directory.join(format!("{}.mp4", output_name));

        let status = Command::new(&self.ae_path)
            .arg("-project")
            .arg(project_path.as_ref())
            .arg("-comp")
            .arg("asset") 
            .arg("-output")
            .arg(&output_path)
            .status()
            .context("Failed to execute aerender")?;

        if !status.success() {
            anyhow::bail!("Rendering failed with status: {}", status);
        }

        Ok(output_path)
    }

    pub async fn cleanup(&self) -> Result<()> {
        let temp_dir = self.output_directory.join("temp");
        if temp_dir.exists() {
            tokio::fs::remove_dir_all(temp_dir)
                .await
                .context("Failed to cleanup temp directory")?;
        }
        Ok(())
    }

    pub async fn process_video<P: AsRef<Path>>(
        &self,
        json_path: P,
        output_name: &str,
    ) -> Result<PathBuf> {
        let data = self.load_data(json_path).await?;
        let assets = self.get_assets(&data.assets).await?;
        let project_path = self.prepare_project(&assets).await?;
        let output_path = self.render_video(&project_path, output_name).await?;
        self.cleanup().await?;

        Ok(output_path)
    }
}

use log::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let video_automation = VideoAutomation::new(
        "Teen Patti Video 2.aep",
        "assets",
        "/",
        "C:/Program Files/Adobe/Adobe After Effects/aerender.exe",
    );

    match video_automation
        .process_video("data.json", "output_render")
        .await
    {
        Ok(output_path) => {
            info!("Video successfully created at: {}", output_path.display());
            Ok(())
        }
        Err(e) => {
            error!("Failed to create video: {}", e);
            Err(e)
        }
    }
}

