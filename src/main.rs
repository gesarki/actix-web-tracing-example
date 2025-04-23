use actix_web::{web, App, HttpResponse, HttpServer, Responder, get, post};
use actix_web_opentelemetry::RequestTracing;
use opentelemetry::global;
use opentelemetry::sdk::propagation::TraceContextPropagator;
use opentelemetry_otlp::WithExportConfig;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Mutex;
use tracing::{info, instrument};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Data structures using Serde for JSON serialization/deserialization
#[derive(Serialize, Deserialize, Clone)]
struct User {
    id: u32,
    name: String,
    email: String,
}

#[derive(Deserialize)]
struct CreateUser {
    name: String,
    email: String,
}

// In-memory database (for demonstration)
struct AppState {
    users: Vec<User>,
    user_counter: u32,
}

// Handler for GET /
#[get("/")]
#[instrument(name = "hello_handler", fields(service = "actix_example"))]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Hello, actix-web!")
}

// Handler for GET /users
#[get("/users")]
#[instrument(name = "get_users_handler", skip(data), fields(service = "actix_example"))]
async fn get_users(data: web::Data<Mutex<AppState>>) -> impl Responder {
    info!("Fetching all users");

    let app_state = match data.lock() {
        Ok(state) => state,
        Err(_) => {
            info!("Failed to lock application state");
            return HttpResponse::InternalServerError().body("Failed to lock application state");
        }
    };
    
    let users = app_state.users.clone();
    let user_count = users.len();
    info!(user_count = user_count, "Successfully fetched users");
    

    HttpResponse::Ok().json(users)
}

// Handler for GET /users/{id}
#[get("/users/{id}")]
#[instrument(name = "get_user_handler", skip(data), fields(service = "actix_example"))]
async fn get_user(path: web::Path<u32>, data: web::Data<Mutex<AppState>>) -> impl Responder {
    let user_id = path.into_inner();
    info!(user_id = user_id, "Looking up user by ID");

    
    let app_state = match data.lock() {
        Ok(state) => state,
        Err(_) => {
            info!("Failed to lock application state");
            return HttpResponse::InternalServerError().body("Failed to lock application state");
        }
    };
    
    match app_state.users.iter().find(|u| u.id == user_id) {
        Some(user) => {
            info!(user_id = user_id, "User found");
            HttpResponse::Ok().json(user.clone())
        },
        None => {
            info!(user_id = user_id, "User not found");
            HttpResponse::NotFound().body(format!("User with ID {} not found", user_id))
        }
    }
}

// Handler for POST /users
#[post("/users")]
#[instrument(name = "create_user_handler", skip(user, data), fields(service = "actix_example"))]
async fn create_user(user: web::Json<CreateUser>, data: web::Data<Mutex<AppState>>) -> impl Responder {
    info!(name = %user.name, email = %user.email, "Creating new user");

    // Lock the mutex to get exclusive access to app state
    let mut app_state = match data.lock() {
        Ok(state) => state,
        Err(_) => {
            info!("Failed to lock application state");
            return HttpResponse::InternalServerError().body("Failed to lock application state");
        }
    };
    
    // Create a new user with auto-incremented ID
    let user_id = app_state.user_counter + 1;
    let new_user = User {
        id: user_id,
        name: user.name.clone(),
        email: user.email.clone(),
    };
    
    // Update the shared state
    app_state.users.push(new_user.clone());
    app_state.user_counter = user_id;

    info!(user_id = user_id, "User created successfully");
    
    // Return the created user with 201 Created status
    HttpResponse::Created().json(new_user)
}

// Get env var from environment variable or default
fn get_env_or_default(env_var: &str, default: &str) -> String {
    let result = env::var(env_var)
        .unwrap_or_else(|_| default.to_string());
    result
}


// Initialize OpenTelemetry with OTLP exporter
fn init_telemetry() -> opentelemetry::sdk::trace::Tracer {
    global::set_text_map_propagator(TraceContextPropagator::new());
    
    // Set up the OTLP exporter
    opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic() // Using gRPC protocol
                .with_endpoint(
                    get_env_or_default("OTLP_ENDPOINT","http://localhost:4317")
                )
        )
        .with_trace_config(
            opentelemetry_sdk::trace::config()
                .with_resource(opentelemetry_sdk::Resource::new(vec![
                    opentelemetry::KeyValue::new("service.name", "actix-web-server"),
                    opentelemetry::KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                    opentelemetry::KeyValue::new("deployment.environment", "development"),
                ]))
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .expect("Failed to install OpenTelemetry tracer")
}


#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize OpenTelemetry
    let tracer = init_telemetry();

    // Initialize tracing subscriber with OpenTelemetry
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("info"))
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .with(tracing_bunyan_formatter::BunyanFormattingLayer::new(
            "actix-web-server".into(), std::io::stdout,
        ))
        .init();
    
    info!("Tracing initialized");
    info!("Sending traces to: {}", get_env_or_default("OTLP_ENDPOINT", "http://localhost:4317"));

    // Initialize application state with Mutex for thread safety
    let app_state = web::Data::new(Mutex::new(AppState {
        users: vec![
            User { id: 1, name: "Alice".to_string(), email: "alice@example.com".to_string() },
            User { id: 2, name: "Bob".to_string(), email: "bob@example.com".to_string() },
        ],
        user_counter: 2,
    }));
    
    info!("Starting HTTP server at http://127.0.0.1:8080");
    
    // Create and start the HTTP server
    let server = HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .wrap(RequestTracing::new()) // Add OpenTelemetry middleware
            .service(hello)
            .service(get_users)
            .service(get_user)
            .service(create_user)
    })
    .bind(("127.0.0.1", 8080))?
    .run();

    info!("Server started");

    // Ensure we flush the tracer when the server stops
    let server_handle = server.handle();
    ctrlc::set_handler(move || {
        info!("Shutting down server");
        server_handle.stop(true);;
        global::shutdown_tracer_provider();
    }).expect("Failed to set Ctrl-C handler");
    
    server.await?;

    // Shut down tracer provider
    global::shutdown_tracer_provider();
    Ok(())

}