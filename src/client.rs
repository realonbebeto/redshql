use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect("postgres://postgres@localhost:5439/public")
        .await
        .unwrap();

    sqlx::query("SELECT 1").execute(&pool).await.unwrap();

    println!("Done");
}
