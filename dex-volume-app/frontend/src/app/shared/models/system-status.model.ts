export interface SystemStatus {
    health: {
        status: string;
        checks: { db: boolean; nats: boolean; redis: boolean };
    };
    workers: {
        sol: { active: number; instances: string[] };
        evm: { active: number; instances: string[] };
    };
    ws_clients: number;
}
