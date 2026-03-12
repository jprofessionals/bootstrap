use std::path::Path;

use axum::{
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

// ---------------------------------------------------------------------------
// CacheMode
// ---------------------------------------------------------------------------

/// Controls the `Cache-Control` header for served static files.
pub enum CacheMode {
    /// Short-lived cache for development: `max-age=60`.
    Development,
    /// Long-lived immutable cache for fingerprinted assets:
    /// `max-age=31536000, immutable`.
    Fingerprinted,
    /// No caching: `no-cache`.
    NoCache,
}

// ---------------------------------------------------------------------------
// serve_static
// ---------------------------------------------------------------------------

/// Serve a static file from disk with ETag support and configurable caching.
///
/// The ETag is based on the file's modification time and size, which is
/// sufficient for development and avoids the cost of hashing file contents.
///
/// If the client sends an `If-None-Match` header that matches the current
/// ETag, a 304 Not Modified response is returned.
pub async fn serve_static(
    path: &Path,
    cache_mode: CacheMode,
    headers: &HeaderMap,
) -> Result<Response, StatusCode> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let content = tokio::fs::read(path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let mtime = metadata
        .modified()
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        })
        .unwrap_or(0);
    let size = metadata.len();
    let etag = format!("\"{}-{}\"", mtime, size);

    // Check If-None-Match
    if let Some(inm) = headers.get(header::IF_NONE_MATCH) {
        if let Ok(val) = inm.to_str() {
            if val == etag {
                return Ok(StatusCode::NOT_MODIFIED.into_response());
            }
        }
    }

    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();

    let cache_control = match cache_mode {
        CacheMode::Development => "max-age=60",
        CacheMode::Fingerprinted => "max-age=31536000, immutable",
        CacheMode::NoCache => "no-cache",
    };

    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, cache_control.to_string()),
        ],
        content,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serve_static_returns_file_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let headers = HeaderMap::new();
        let resp = serve_static(&file_path, CacheMode::NoCache, &headers)
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn serve_static_returns_404_for_missing_file() {
        let headers = HeaderMap::new();
        let result = serve_static(
            Path::new("/nonexistent/file.txt"),
            CacheMode::NoCache,
            &headers,
        )
        .await;

        assert_eq!(result.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn serve_static_returns_304_on_matching_etag() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("cached.txt");
        std::fs::write(&file_path, "cached content").unwrap();

        // First request to get the ETag
        let headers = HeaderMap::new();
        let resp = serve_static(&file_path, CacheMode::NoCache, &headers)
            .await
            .unwrap();
        let etag = resp
            .headers()
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        // Second request with matching If-None-Match
        let mut headers2 = HeaderMap::new();
        headers2.insert(header::IF_NONE_MATCH, etag.parse().unwrap());
        let resp2 = serve_static(&file_path, CacheMode::NoCache, &headers2)
            .await
            .unwrap();

        assert_eq!(resp2.status(), StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn serve_static_cache_mode_development() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("dev.js");
        std::fs::write(&file_path, "console.log('hi')").unwrap();

        let headers = HeaderMap::new();
        let resp = serve_static(&file_path, CacheMode::Development, &headers)
            .await
            .unwrap();

        let cc = resp
            .headers()
            .get(header::CACHE_CONTROL)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(cc, "max-age=60");
    }

    #[tokio::test]
    async fn serve_static_cache_mode_fingerprinted() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("app.abc123.js");
        std::fs::write(&file_path, "minified").unwrap();

        let headers = HeaderMap::new();
        let resp = serve_static(&file_path, CacheMode::Fingerprinted, &headers)
            .await
            .unwrap();

        let cc = resp
            .headers()
            .get(header::CACHE_CONTROL)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(cc, "max-age=31536000, immutable");
    }

    #[tokio::test]
    async fn serve_static_cache_mode_nocache() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("api.json");
        std::fs::write(&file_path, "{}").unwrap();

        let headers = HeaderMap::new();
        let resp = serve_static(&file_path, CacheMode::NoCache, &headers)
            .await
            .unwrap();

        let cc = resp
            .headers()
            .get(header::CACHE_CONTROL)
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(cc, "no-cache");
    }

    #[tokio::test]
    async fn serve_static_sets_correct_mime_type() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("style.css");
        std::fs::write(&file_path, "body { color: red }").unwrap();

        let headers = HeaderMap::new();
        let resp = serve_static(&file_path, CacheMode::NoCache, &headers)
            .await
            .unwrap();

        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("css"));
    }

    #[tokio::test]
    async fn serve_static_etag_format() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let headers = HeaderMap::new();
        let resp = serve_static(&file_path, CacheMode::NoCache, &headers)
            .await
            .unwrap();

        let etag = resp.headers().get(header::ETAG).unwrap().to_str().unwrap();
        // ETag format: "<mtime>-<size>"
        assert!(etag.starts_with('"'));
        assert!(etag.ends_with('"'));
        assert!(etag.contains('-'));
    }

    #[tokio::test]
    async fn serve_static_non_matching_etag_returns_200() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, "\"wrong-etag\"".parse().unwrap());
        let resp = serve_static(&file_path, CacheMode::NoCache, &headers)
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }
}
