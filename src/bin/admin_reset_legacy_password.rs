use chat_room_prog2::controller::{
    forms::admin_reset_legacy_password, rooms::prepare_database_schema,
};
use sea_orm::Database;
use std::{env, io::Write};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Napaka: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let legacy_username = match args.next() {
        Some(argument) if argument == "-h" || argument == "--help" => {
            print_usage();
            return Ok(());
        }
        Some(username) => username,
        None => {
            print_usage();
            return Err("Manjka uporabniško ime legacy računa.".to_string());
        }
    };
    let new_username = args.next();
    if args.next().is_some() {
        print_usage();
        return Err("Podanih je preveč argumentov.".to_string());
    }

    dotenvy::dotenv().ok();
    let database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://./chat.db?mode=rwc".to_string());
    let db = Database::connect(&database_url)
        .await
        .map_err(|e| format!("Povezava z bazo ni uspela: {e}"))?;
    prepare_database_schema(&db)
        .await
        .map_err(|e| e.to_string())?;

    println!("Administratorski reset legacy računa '{legacy_username}'.");
    println!("Geslo bo med tipkanjem vidno, vendar se ne shrani v zgodovino ukazov.");
    let password = read_line("Novo geslo: ")?;
    let confirmation = read_line("Ponovi novo geslo: ")?;
    if password != confirmation {
        return Err("Gesli se ne ujemata.".to_string());
    }

    let updated =
        admin_reset_legacy_password(&db, &legacy_username, new_username.as_deref(), &password)
            .await?;

    println!(
        "Geslo legacy računa '{}' je uspešno nastavljeno.",
        updated.username
    );
    Ok(())
}

fn read_line(prompt: &str) -> Result<String, String> {
    print!("{prompt}");
    std::io::stdout()
        .flush()
        .map_err(|e| format!("Izpis poziva ni uspel: {e}"))?;

    let mut value = String::new();
    std::io::stdin()
        .read_line(&mut value)
        .map_err(|e| format!("Branje vnosa ni uspelo: {e}"))?;
    while matches!(value.chars().last(), Some('\n' | '\r')) {
        value.pop();
    }
    Ok(value)
}

fn print_usage() {
    println!("Uporaba: cargo run --bin admin_reset_legacy_password -- <staro_ime> [novo_ime]");
}
