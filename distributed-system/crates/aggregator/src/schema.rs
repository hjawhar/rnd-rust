diesel::table! {
    results (id) {
        id -> Uuid,
        request_id -> Uuid,
        task_type -> Text,
        payload -> Jsonb,
        enrichment -> Nullable<Jsonb>,
        processed_at -> Timestamptz,
        processor_id -> Text,
    }
}
