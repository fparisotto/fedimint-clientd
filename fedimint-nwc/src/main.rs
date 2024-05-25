use anyhow::Result;
use clap::Parser;
use nostr_sdk::{JsonUtil, Kind, RelayPoolNotification};
use tokio::pin;
use tracing::{error, info};

pub mod config;
pub mod managers;
pub mod nwc;
pub mod services;
pub mod state;

use state::AppState;

use crate::config::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    dotenv::dotenv().ok();

    let cli = Cli::parse();
    let state = AppState::new(cli).await?;

    // Connect to the relay pool and broadcast the info event on startup
    state.nostr_service.connect().await;
    state
        .nostr_service
        .broadcast_info_event(&state.key_manager)
        .await?;

    // Start the event loop
    event_loop(state.clone()).await?;

    Ok(())
}

/// Event loop that listens for nostr wallet connect events and handles them
async fn event_loop(state: AppState) -> Result<()> {
    // Handle ctrl+c to gracefully shutdown the event loop
    let ctrl_c = tokio::signal::ctrl_c();
    pin!(ctrl_c);

    let mut notifications = state.nostr_service.notifications();
    info!("Listening for events...");
    loop {
        tokio::select! {
            _ = &mut ctrl_c => {
                info!("Ctrl+C received. Waiting for active requests to complete...");
                state.wait_for_active_requests().await;
                info!("All active requests completed.");
                break;
            },
            notification = notifications.recv() => {
                match notification {
                    Ok(notification) => match notification {
                        RelayPoolNotification::Event { event, .. } => {
                            if event.kind == Kind::WalletConnectRequest
                                && event.pubkey == state.key_manager.user_keys().public_key()
                                && event.verify().is_ok() {
                                    info!("Received event: {}", event.as_json());
                                    state.handle_event(*event).await
                            } else {
                                error!("Invalid nwc event: {}", event.as_json());
                            }
                        },
                        RelayPoolNotification::Shutdown => {
                            info!("Relay pool shutdown");
                            break;
                        },
                        _ => {
                            error!("Unhandled relay pool notification: {notification:?}");
                        }
                    },
                    Err(_) => {},
                }
            }
        }
    }

    state.nostr_service.disconnect().await?;
    Ok(())
}
