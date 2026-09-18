use futures::{future::join_all, lock::Mutex, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressState, ProgressStyle};
use std::{fmt::Write, sync::Arc};
use tokio::{fs::File, io::AsyncWriteExt};

pub async fn download_file(url: &str) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    match std::fs::exists("./downloads") {
        Ok(exists) => {
            if !exists {
                let created_dir = std::fs::create_dir("./downloads");
                if let Err(err) = created_dir {
                    panic!("Failed to create downloads directory {:#?}", err);
                }
            }
        }
        Err(err) => {
            panic!("Couldn't resolve downloads folder {:#?}", err);
        }
    }

    let file_name = format!("{}", url.split("/").last().unwrap_or("Unknown"));
    let mut file = File::create(format!("./downloads/{}", file_name)).await?;
    let stream = reqwest::get(url).await?;
    let file_size = match stream.headers().get("content-length") {
        Some(val) => val.to_str()?.parse::<u64>()?,
        None => 0,
    };
    let pb = ProgressBar::new(file_size);
    pb.set_message(format!("Starting download {}", file_name));
    pb.set_style(
        ProgressStyle::with_template(
            "{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})",
        ).unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
    );
    let mut downloaded = 0;
    let mut b_stream = stream.bytes_stream();
    while let Some(chunk_result) = b_stream.next().await {
        pb.set_message(format!("Downoading {}", file_name));
        let chunk = chunk_result?;
        downloaded = downloaded + chunk.len() as u64;
        pb.set_position(downloaded + chunk.len() as u64);
        file.write_all(&chunk).await?;
    }
    pb.set_length(downloaded);
    file.flush().await?;
    pb.finish_with_message(format!("Successfully downloaded {}", file_name));
    Ok(true)
}

pub async fn download_files(
    urls: Vec<&str>,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let multi_progress = MultiProgress::new();
    match std::fs::exists("./downloads") {
        Ok(exists) => {
            if !exists {
                let created_dir = std::fs::create_dir("./downloads");
                if let Err(err) = created_dir {
                    panic!("Failed to create downloads directory {:#?}", err);
                }
            }
        }
        Err(err) => {
            panic!("Couldn't resolve downloads folder {:#?}", err);
        }
    }

    let multi_progress = Arc::new(Mutex::new(multi_progress));
    let mut join_handlers = vec![];
    for url in urls {
        let file_name = format!("{}", url.split("/").last().unwrap_or("Unknown"));
        let mut file = File::create(format!("./downloads/{}", file_name)).await?;
        let stream = reqwest::get(url).await?;
        let file_size = match stream.headers().get("content-length") {
            Some(val) => val.to_str()?.parse::<u64>()?,
            None => 0,
        };

        let multi_progress = multi_progress.clone();
        join_handlers.push(tokio::spawn(async move {
            let m = multi_progress.lock().await;
            let pb = ProgressBar::new(file_size);
            pb.set_style(ProgressStyle::with_template("{msg} {spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})").unwrap()
                    .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap()));
            let pb2 = pb.clone();
            m.add(pb2);
            drop(m);
            let mut downloaded = 0;
            let mut b_stream = stream.bytes_stream();
            pb.set_message(format!("Starting download {}", file_name));
            while let Some(chunk_result) = b_stream.next().await {
                pb.set_message(format!("Downoading {}", file_name));
                if let Ok(chunk) = chunk_result {
                    downloaded = downloaded + chunk.len() as u64;
                    pb.clone().set_position(downloaded + chunk.len() as u64);
                    if let Ok(_) = file.write_all(&chunk).await {}
                }
            }
            pb.set_length(downloaded);
            if let Ok(_) = file.flush().await {
                pb.finish_with_message(format!("Successfully downloaded {}", file_name));
            }
        }));
    }

    join_all(join_handlers).await;
    Ok(true)
}
