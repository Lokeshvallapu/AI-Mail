use bcrypt::{hash, verify};
use sqlx::Row;
use reqwest::Client;
use serde_json::json;
use tower_http::cors::CorsLayer;
use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Json,
    Router,
};

use serde::{Deserialize, Serialize};

use sqlx::{
    postgres::PgPoolOptions,
    PgPool,
};

use dotenvy::dotenv;
use std::env;

use jsonwebtoken::{
    encode,
    decode,
    Header,
    Validation,
    EncodingKey,
    DecodingKey,
};

use chrono::{
    Utc,
    Duration,
};

use lettre::{
    Message,
    SmtpTransport,
    Transport,
    transport::smtp::authentication::Credentials,
};

#[derive(Deserialize)]
struct RegisterUser {
    name: String,
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginUser {
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct SendEmailRequest {
    to: String,
    subject: String,
    body: String,
}

#[derive(Deserialize)]
struct AiEmailRequest {
    prompt: String,
}

#[derive(Serialize)]
struct EmailHistory {
    id: i32,
    prompt: String,
    generated_email: String,
    created_at: String,
}

#[derive(Serialize)]
struct ApiResponse {
    message: String,
    token: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

async fn root() -> &'static str {
    "AI Mail Backend Running"
}

async fn register_user(
    State(db): State<PgPool>,
    Json(payload): Json<RegisterUser>,
) -> Json<ApiResponse> {

    let query = "
        INSERT INTO users (name, email, password)
        VALUES ($1, $2, $3)
    ";

        let hashed_password =
    hash(&payload.password, 10).unwrap();

sqlx::query(query)
    .bind(&payload.name)
    .bind(&payload.email)
    .bind(&hashed_password)
        .execute(&db)
        .await
        .unwrap();

    Json(ApiResponse {
        message: "User registered successfully!".to_string(),
        token: None,
    })
}

 async fn login_user(
    State(db): State<PgPool>,
    Json(payload): Json<LoginUser>,
) -> Json<ApiResponse> {

    let row = sqlx::query(
        "SELECT password FROM users WHERE email = $1",
    )
    .bind(&payload.email)
    .fetch_one(&db)
    .await;

    match row {

        Ok(user) => {

            let valid = verify(
                &payload.password,
                user.get::<String, _>("password").as_str(),
            ).unwrap();

            if valid {
                let expiration =
                    Utc::now()
                    + Duration::hours(24);

                let claims = Claims {
                    sub: payload.email.clone(),
                    exp: expiration.timestamp() as usize,
               };

              let token = encode(
                  &Header::default(),
                  &claims,
                  &EncodingKey::from_secret(
                      "mysecretkey".as_ref()
               ),
           ).unwrap();

          Json(ApiResponse {
              message: "Login successful!".to_string(),
              token: Some(token),
   })    

            } else {

                Json(ApiResponse {
                    message: "Invalid password!".to_string(),
                    token: None,
                })
            }
        }

        Err(_) => {

            Json(ApiResponse {
                message: "User not found!".to_string(),
                token: None,
            })
        }
    }
}

async fn profile(
    headers: HeaderMap,
) -> Json<ApiResponse> {

    let token = headers
        .get("Authorization")
        .unwrap()
        .to_str()
        .unwrap()
        .replace("Bearer ", "");

    let decoded = decode::<Claims>(
        &token,
        &DecodingKey::from_secret(
            "mysecretkey".as_ref()
        ),
        &Validation::default(),
    );

    match decoded {

        Ok(data) => {

            Json(ApiResponse {
                message: format!(
                    "Welcome {}",
                    data.claims.sub
                ),
                token: None,
            })
        }

        Err(_) => {

            Json(ApiResponse {
                message: "Invalid token!".to_string(),
                token: None,
            })
        }
    }
}

async fn send_email(
    State(pool): State<PgPool>,
    Json(payload): Json<SendEmailRequest>,
) -> Json<ApiResponse> {

    let smtp_email =
        env::var("SMTP_EMAIL").unwrap();

    let smtp_password =
        env::var("SMTP_PASSWORD").unwrap();

    let email = Message::builder()
        .from(smtp_email.parse().unwrap())
        .to(payload.to.parse().unwrap())
        .subject(&payload.subject)
        .body(payload.body.clone())
        .unwrap();

    let creds = Credentials::new(
        smtp_email.clone(),
        smtp_password,
    );

    let mailer = SmtpTransport::relay(
        "smtp.gmail.com"
    )
    .unwrap()
    .credentials(creds)
    .build();

    let result = mailer.send(&email);

    match result {

        Ok(_) => {

            sqlx::query(
                "
                INSERT INTO sent_emails
                (to_email, subject, body)
                VALUES ($1, $2, $3)
                "
            )
            .bind(&payload.to)
            .bind(&payload.subject)
            .bind(&payload.body)
            .execute(&pool)
            .await
            .unwrap();

            Json(ApiResponse {
                message: "Email sent successfully!"
                    .to_string(),
                token: None,
            })
        }

        Err(_) => Json(ApiResponse {
            message: "Failed to send email!"
                .to_string(),
            token: None,
        }),
    }
}

async fn generate_email(
    State(db): State<PgPool>,
    Json(payload): Json<AiEmailRequest>,
) -> Json<ApiResponse> {

   println!("{:?}", env::var("GROQ_API_KEY"));

let api_key = env::var("GROQ_API_KEY")
    .expect("GROQ_API_KEY not found");

let client = Client::new();

let response = client
    .post("https://api.groq.com/openai/v1/chat/completions")
    .header("Authorization", format!("Bearer {}", api_key))
    .header("Content-Type", "application/json")
    .json(&json!({
        "model": "llama-3.1-8b-instant",
        "messages": [
            {
                "role": "user",
                "content": payload.prompt
            }
        ]
    }))
    .send()
    .await
    .unwrap();

let response_json: serde_json::Value =
    response.json().await.unwrap();

println!("{:#?}", response_json);

let generated_email = response_json["choices"][0]["message"]["content"]
    .as_str()
    .unwrap_or("No response")
    .to_string();

    sqlx::query(
    "
    INSERT INTO emails
    (prompt, generated_email)
    VALUES ($1, $2)
    "
)
.bind(&payload.prompt)
.bind(&generated_email)
.execute(&db)
.await
.unwrap();

Json(ApiResponse {
    message: generated_email,
    token: None,
})
}

async fn get_emails(
    State(db): State<PgPool>,
) -> Json<Vec<EmailHistory>> {

    let rows = sqlx::query(
        "
        SELECT id, prompt,
        generated_email,
        created_at
        FROM emails
        ORDER BY id DESC
        "
    )
    .fetch_all(&db)
    .await
    .unwrap();

    let emails = rows
        .into_iter()
        .map(|row| EmailHistory {

            id: row.get("id"),

            prompt: row.get("prompt"),

            generated_email:
                row.get("generated_email"),

         created_at:
            row.get::<chrono::NaiveDateTime, _>(
                "created_at"
             ).to_string(),
        })
        .collect();

    Json(emails)
}

async fn get_sent_emails(
    State(pool): State<PgPool>,
) -> Json<Vec<serde_json::Value>> {

    let rows = sqlx::query(
        "
        SELECT to_email, subject, body
        FROM sent_emails
        ORDER BY id DESC
        "
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let emails: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "to": row.get::<String, _>("to_email"),
                "subject": row.get::<String, _>("subject"),
                "body": row.get::<String, _>("body"),
            })
        })
        .collect();

    Json(emails)
}


#[tokio::main]
async fn main() {

    dotenv().ok();

    let database_url =
        env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to database");

    println!("Database connected successfully!");

    let app = Router::new()
        .route("/", get(root))
        .route("/register", post(register_user))
        .route("/login", post(login_user))
        .route("/profile", get(profile))
        .route("/send-email", post(send_email))
        .route("/generate-email", post(generate_email))
        .route("/emails", get(get_emails))
        .route("/sent-emails", get(get_sent_emails))
        .layer(CorsLayer::permissive())
        .with_state(db);

    let listener =
        tokio::net::TcpListener::bind("0.0.0.0:3001")
        .await
        .unwrap();

    println!("Server running on http://localhost:3001");

    axum::serve(listener, app)
        .await
        .unwrap();
}