use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use nostr_sdk::{Event, EventId, JsonUtil, Kind};
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use crate::config::Cli;
use crate::managers::KeyManager;
use crate::nwc::{handle_nwc_request, NwcConfig};
use crate::services::{MultiMintService, NostrService};

#[derive(Debug, Clone)]
pub struct AppState {
    pub multimint_service: MultiMintService,
    pub nostr_service: NostrService,
    pub key_manager: KeyManager,
    pub active_requests: Arc<Mutex<BTreeSet<EventId>>>,
    pub nwc_config: NwcConfig,
}

impl AppState {
    pub async fn new(cli: Cli) -> Result<Self, anyhow::Error> {
        let key_manager = KeyManager::new(&cli.keys_file)?;
        let multimint_service = MultiMintService::new(cli.db_path).await?;
        let nostr_service = NostrService::new(&key_manager, &cli.relays).await?;

        let active_requests = Arc::new(Mutex::new(BTreeSet::new()));
        let nwc_config = NwcConfig {
            max_amount: cli.max_amount,
            daily_limit: cli.daily_limit,
        };

        Ok(Self {
            multimint_service,
            nostr_service,
            key_manager,
            active_requests,
            nwc_config,
        })
    }

    pub async fn init(&mut self, cli: &Cli) -> Result<(), anyhow::Error> {
        self.multimint_service
            .init_multimint(&cli.invite_code, cli.manual_secret.clone())
            .await?;
        Ok(())
    }

    pub async fn wait_for_active_requests(&self) {
        let requests = self.active_requests.lock().await;
        loop {
            if requests.is_empty() {
                break;
            }
            debug!("Waiting for {} requests to complete...", requests.len());
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// Adds nwc events to active requests set while waiting for them to
    /// complete so they can finish processing before a shutdown.
    pub async fn handle_event(&self, event: Event) {
        if event.kind == Kind::WalletConnectRequest && event.verify().is_ok() {
            info!("Received event: {}", event.as_json());
            let event_id = event.id;
            self.active_requests.lock().await.insert(event_id);

            match tokio::time::timeout(Duration::from_secs(60), handle_nwc_request(&self, event))
                .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => error!("Error processing request: {e}"),
                Err(e) => error!("Timeout error: {e}"),
            }

            self.active_requests.lock().await.remove(&event_id);
        } else {
            error!("Invalid event: {}", event.as_json());
        }
    }
}
