//! Temporary Qdrant round-trip smoke test.
//! Run: cargo run -p rag-storage-qdrant --example roundtrip_smoke
//! Requires a running Qdrant server (docker compose up -d qdrant).

use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, QueryPointsBuilder, UpsertPointsBuilder,
    VectorParamsBuilder,
};
use qdrant_client::Qdrant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("RAG_QDRANT_URL").unwrap_or_else(|_| "http://localhost:6334".into());
    let client = Qdrant::from_url(&url).build()?;

    let collection = "smoke_roundtrip";
    let vector = vec![0.1_f32, 0.2, 0.3, 0.4];

    // 1. WRITE: create collection
    client
        .create_collection(
            CreateCollectionBuilder::new(collection)
                .vectors_config(VectorParamsBuilder::new(4, Distance::Cosine)),
        )
        .await?;
    println!("[1/3] collection created: {collection}");

    // 2. WRITE: upsert a point
    let points = vec![PointStruct::new(
        1,
        vector.clone(),
        [("text", "hello qdrant round-trip".into())],
    )];
    client
        .upsert_points(UpsertPointsBuilder::new(collection, points))
        .await?;
    println!("[2/3] point upserted");

    // 3. SEARCH: query with the same vector -> expect the point back, score ~1.0
    let res = client
        .query(
            QueryPointsBuilder::new(collection)
                .query(vector.clone())
                .limit(3)
                .with_payload(true),
        )
        .await?;
    println!("[3/3] search results: {res:#?}");

    let hit = res.result.first().expect("expected at least one hit");
    assert_eq!(hit.id, Some(1.into()), "expected point id 1");
    let score = hit.score;
    assert!(
        (score - 1.0).abs() < 1e-4,
        "expected score ~1.0 for exact vector match, got {score}"
    );

    println!("ROUND-TRIP OK: write + search against {url} succeeded (score={score})");
    Ok(())
}