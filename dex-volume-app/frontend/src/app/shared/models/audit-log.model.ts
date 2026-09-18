export type AuditLog = {
    id: number,
    user_id: number,
    project_id: number | null,
    action: string,
    details: Record<string, any> | null,
    created_at: {
        nanos_since_epoch: number,
        secs_since_epoch: number
    }
}
