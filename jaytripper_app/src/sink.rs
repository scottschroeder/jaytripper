use async_trait::async_trait;
use jaytripper_core::{MovementEvent, MovementEventSink};

use crate::{AppError, app::AppRuntime};

#[derive(Clone)]
pub(crate) struct AppMovementSink {
    app: AppRuntime,
}

impl AppMovementSink {
    pub(crate) fn new(app: AppRuntime) -> Self {
        Self { app }
    }
}

#[async_trait]
impl MovementEventSink for AppMovementSink {
    type Error = AppError;

    async fn emit_movement(&self, event: MovementEvent) -> Result<(), Self::Error> {
        self.app.store().append_movement_event(&event).await?;
        self.app.catch_up_projection_from_store().await
    }
}
