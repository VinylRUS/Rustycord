//! Мост протокольного ядра (tokio) и GPUI (свой executor):
//!
//! * единственный на процесс tokio-рантайм — на нём живут gateway/voice/REST
//!   задачи ядра (в Tauri эту роль играл tauri::async_runtime);
//! * [`GpuiSink`] — реализация `EventSink`: события ядра уходят в
//!   unbounded-канал, который качает foreground-задача GPUI и раздаёт их
//!   в сущность [`crate::state::AppModel`] (один и тот же канал несёт и
//!   `discord://…` события ядра, и `ui://…` результаты команд UI —
//!   tokio-таскам нельзя трогать GPUI-сущности напрямую).

use std::sync::OnceLock;

use rustycord_core::EventSink;
use serde_json::Value;
use tokio::sync::mpsc;

/// Глобальный tokio-рантайм (мультипоточный; живёт до конца процесса).
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

/// Спавн задачи на протокольном рантайме (fire-and-forget).
pub fn spawn_tokio<F>(task: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    runtime().spawn(task);
}

/// Событийный канал «ядро → GPUI»: (имя события, JSON-пейлоад).
pub type EventTx = mpsc::UnboundedSender<(String, Value)>;
pub type EventRx = mpsc::UnboundedReceiver<(String, Value)>;

pub fn event_channel() -> (EventTx, EventRx) {
    mpsc::unbounded_channel()
}

/// Реализация `EventSink` для GPUI: кладёт событие в канал к pump-задаче.
#[derive(Clone)]
pub struct GpuiSink(pub EventTx);

impl EventSink for GpuiSink {
    fn emit(&self, event: &str, payload: Value) {
        // Канал unbounded: send не блокирует event loop ядра; если pump
        // умер (окно закрыто) — тихо игнорируем.
        let _ = self.0.send((event.to_string(), payload));
    }
}

/// Отправить событие в UI-канал из tokio-задачи (результаты REST-команд).
pub fn ui_emit(tx: &EventTx, event: &str, payload: Value) {
    let _ = tx.send((event.to_string(), payload));
}
