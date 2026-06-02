use super::IngressTurnEnvelope;

pub(crate) fn annotate_message_ingress_metadata(
    msg: &mut crate::SessionMessage,
    ingress: Option<&IngressTurnEnvelope>,
) {
    let Some(ingress) = ingress else {
        return;
    };

    msg.metadata.insert(
        "ingress_source".to_string(),
        serde_json::json!(&ingress.source),
    );
    msg.metadata.insert(
        "ingress_turn_id".to_string(),
        serde_json::json!(&ingress.turn_id),
    );
    msg.metadata.insert(
        "ingress_stabilization".to_string(),
        serde_json::json!(&ingress.stabilization),
    );

    if let Some(key) = ingress.idempotency_key.as_deref() {
        msg.metadata.insert(
            "ingress_idempotency_key".to_string(),
            serde_json::json!(key),
        );
    }
    if let Some(context_key) = ingress.context_key.as_deref() {
        msg.metadata.insert(
            "ingress_context_key".to_string(),
            serde_json::json!(context_key),
        );
    }
    if let Some(stage_id) = ingress.scheduler_stage_id.as_deref() {
        msg.metadata.insert(
            "ingress_scheduler_stage_id".to_string(),
            serde_json::json!(stage_id),
        );
    }
    if let Some(origin) = ingress.source_origin {
        msg.metadata.insert(
            rocode_types::MESSAGE_SOURCE_ORIGIN_KEY.to_string(),
            serde_json::to_value(origin).unwrap_or_default(),
        );
    }
    if let Some(surface) = ingress.source_surface {
        msg.metadata.insert(
            rocode_types::MESSAGE_SOURCE_SURFACE_KEY.to_string(),
            serde_json::to_value(surface).unwrap_or_default(),
        );
    }
    if let Some(origin) = ingress.source_origin {
        let (admission, authority_class) = rocode_types::origin_to_admission_authority(origin);
        rocode_types::apply_message_admission_metadata(
            &mut msg.metadata,
            admission,
            authority_class,
        );
    }
}
