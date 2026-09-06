use std::sync::Mutex;

use tracing::{Event, Metadata, Subscriber, field::Visit, span};

#[derive(Default)]
pub(super) struct LogCapture(Mutex<Vec<String>>);

impl LogCapture {
    pub(super) fn assert_cleanup_failure_recorded(&self) {
        let events = self.0.lock().unwrap();
        assert!(
            events.iter().any(|event| {
                event.contains("preserving the operation error")
                    && event.contains("roll back a failed SQLite transaction")
                    && event.contains("cannot rollback - no transaction is active")
            }),
            "secondary cleanup failure was not recorded: {events:?}"
        );
    }
}

impl Subscriber for LogCapture {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "agena_storage_sqlite::transaction"
    }

    fn new_span(&self, _attributes: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}
    fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}
    fn enter(&self, _span: &span::Id) {}
    fn exit(&self, _span: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        struct Fields(String);

        impl Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                write!(&mut self.0, " {}={value:?}", field.name()).unwrap();
            }
        }

        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
}
