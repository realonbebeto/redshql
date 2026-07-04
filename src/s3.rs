use anyhow::Context;
use s3::{Bucket, Region, creds::Credentials};

#[allow(unused)]
#[derive(Clone)]
pub struct S3Client {
    bucket: String,
    inner: Box<Bucket>,
}

impl S3Client {
    pub async fn new(bucket_name: &str) -> anyhow::Result<Self> {
        let endpoint = std::env::var("S3_ENDPOINT").context("S3_ENDPOINT not set")?;
        let access_key = std::env::var("MINIO_ROOT_USER").unwrap_or_else(|_| "minioadmin".into());
        let secret_key =
            std::env::var("MINIO_ROOT_PASSWORD").unwrap_or_else(|_| "minioadmin".into());
        let region = Region::Custom {
            region: std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".into()),
            endpoint,
        };

        let credentials = Credentials::new(
            Some(access_key.as_str()),
            Some(secret_key.as_str()),
            None,
            None,
            None,
        )?;

        let bucket = Bucket::new(bucket_name, region, credentials).map_err(|e| {
            tracing::error!("Failed to create S3 bucket client: {e}");
            anyhow::anyhow!("Failed to setup bucket: {e}")
        })?;

        bucket.with_path_style();

        Ok(S3Client {
            bucket: bucket_name.into(),
            inner: bucket,
        })
    }

    pub async fn get_object(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        let response = self.inner.get_object(key).await?;

        if response.status_code() != 200 {
            tracing::error!(
                "S3 upload returned status {}: {}",
                response.status_code(),
                String::from_utf8_lossy(response.as_slice())
            );
            return Err(anyhow::anyhow!(format!(
                "S3 upload failed with status {}",
                response.status_code()
            )));
        }

        tracing::info!(
            "Retrieved {} bytes to s3://{}/{}",
            response.bytes().len(),
            self.inner.name(),
            key
        );

        Ok(response.bytes().to_vec())
    }

    pub async fn list(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let mut keys = Vec::new();

        let req = self.inner.list(prefix.into(), Some("/".into())).await?;

        for b_result in req {
            let ks = b_result
                .contents
                .into_iter()
                .map(|v| v.key)
                .collect::<Vec<String>>();

            keys.extend(ks);
        }

        Ok(keys)
    }
}

pub fn split_s3_uri(uri: &str) -> anyhow::Result<(String, String)> {
    let rest = uri.strip_prefix("s3://").context("not an s3:// URI")?;
    match rest.split_once('/') {
        Some((b, p)) => Ok((b.to_string(), p.to_string())),
        None => anyhow::bail!("s3 URI missing key/prefix: {uri}"),
    }
}
