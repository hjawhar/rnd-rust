#[cfg(test)]
mod tests {
    use downloader::{download_file, download_files};
    #[tokio::test]
    async fn test_download_single() {
        let test_dl = download_file("https://test2.fibertelecom.it/10MB.zip").await;
        let res = match test_dl {
            Ok(res) => res,
            Err(_) => false,
        };
        assert_eq!(res, true);
    }

    #[tokio::test]
    async fn test_download_multiple() {
        let test_dl = download_files(vec![
            "https://test2.fibertelecom.it/50MB.zip",
            "https://test2.fibertelecom.it/20MB.zip",
            "https://test2.fibertelecom.it/10MB.zip",
            "https://test2.fibertelecom.it/5MB.zip",
        ])
        .await;
        let res = match test_dl {
            Ok(res) => res,
            Err(_) => false,
        };
        assert_eq!(res, true);
    }
}
