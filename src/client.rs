// use s3::{Bucket, Region, creds::Credentials};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect("postgres://postgres@localhost:5439/public")
        .await
        .unwrap();

    // let region = Region::Custom {
    //     region: "us-east-1".into(),
    //     endpoint: "http://localhost:9000".into(),
    // };

    // let credentials = Credentials::new(Some("admin"), Some("minioroot"), None, None, None).unwrap();

    // let bucket = Bucket::new("iggy", region, credentials)
    //     .unwrap()
    //     .with_path_style();

    // let response = bucket.put_object("/test.file", content).await.unwrap();

    // if response.status_code() != 200 {
    //     tracing::error!(
    //         "S3 upload returned status {}: headers={:?} body={:?}",
    //         response.status_code(),
    //         response.headers(),
    //         response.to_string()
    //     );
    // }

    sqlx::query("COPY messages FROM 's3://iggystaging/iggy/messages/019f4265-3ca5-71a1-bf24-5f96913d0a71.parquet' CREDENTIALS 'ACCESS_KEY_ID=admin; SECRET_ACCESS_KEY=password' FORMAT AS PARQUET REGION 'us-east-1';").execute(&pool).await.unwrap();

    // sqlx::query(
    //     "CREATE TABLE IF NOT EXISTS my_table2 (
    //                         id DECIMAL(39, 0) PRIMARY KEY,
    //                         iggy_offset BIGINT,
    //                         iggy_timestamp TIMESTAMP WITH TIME ZONE,
    //                         iggy_stream TEXT,
    //                         iggy_topic TEXT,
    //                         iggy_partition_id INTEGER,
    //                         iggy_checksum BIGINT,
    //                         iggy_origin_timestamp TIMESTAMP WITH TIME ZONE,
    //                         payload VARBYTE,
    //                         created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW())",
    // )
    // .execute(&pool)
    // .await
    // .unwrap();

    println!("Done");
}
