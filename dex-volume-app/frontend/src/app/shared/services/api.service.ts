import { Injectable } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { Observable } from 'rxjs';
import { MatSnackBar } from '@angular/material/snack-bar';
import { environment } from '../../../environments/environment';
import { AddProjectPayload, Project, ProjectStatistics, UpdateProject } from '../models/project.model';
import { ProjectWalletsFinancials } from '../models/wallets.model';
import { ProjectTransactions, Transaction } from '../models/transaction.model';
import { AddUserPayload, ProjectAccess, User } from '../models/user.model';
import { CommonPool } from '../models/token.model';
import { SystemStatus } from '../models/system-status.model';
import { AuditLog } from '../models/audit-log.model';
import { CreatePaymentPayload, CreateSubscriptionPayload, Payment, SubscriptionInfo, SubscriptionWithProject, UpdateSubscriptionPayload, Subscription } from '../models/subscription.model';

@Injectable({
    providedIn: 'root'
})
export class ApiService {
    protected defaultController = `${environment.baseUrl}`;
    constructor(public http: HttpClient, private snackBar: MatSnackBar) {
    }

    getNonce(publicKey: string): Observable<{ nonce: string, address: string }> {
        return this.http.post<{ nonce: string, address: string }>(`${environment.baseUrl}/oauth/nonce`, { address: publicKey });
    }

    authenticate(address: string, nonce: string, signed_nonce: string): Observable<{ jwt: string, refresh_token: string }> {
        return this.http.post<{ jwt: string, refresh_token: string }>(`${environment.baseUrl}/oauth`, {
            address,
            nonce,
            signed_nonce
        });
    }

    refreshToken(refresh_token: string): Observable<{ jwt: string, refresh_token: string }> {
        return this.http.post<{ jwt: string, refresh_token: string }>(`${environment.baseUrl}/oauth/refresh`, { refresh_token });
    }

    logout(): Observable<{ success: boolean }> {
        return this.http.post<{ success: boolean }>(`${environment.baseUrl}/oauth/logout`, {});
    }

    generateWallets(projectId: number, payload: { value: number }): Observable<{ data: string }> {
        return this.http.post<{ data: string }>(`${environment.baseUrl}/projects/${projectId}/wallets/generate`, payload);
    }

    importWallets(projectId: number, payload: { pks: string }): Observable<{ data: string }> {
        return this.http.post<{ data: string }>(`${environment.baseUrl}/projects/${projectId}/wallets/import`, payload);
    }

    deleteWallets(projectId: number, payload: { ids: number[] }): Observable<string> {
        return this.http.post<string>(`${environment.baseUrl}/projects/${projectId}/wallets/delete`, payload);
    }

    exportWallets(projectId: number, payload: { ids: number[] }): Observable<{ data: string }> {
        return this.http.post<{ data: string }>(`${environment.baseUrl}/projects/${projectId}/wallets/export`, payload);
    }

    /**
     * Projects
     */

    addProject(payload: AddProjectPayload): Observable<{ data: Project | null }> {
        return this.http.post<{ data: Project | null }>(`${environment.baseUrl}/projects`, payload);
    }

    getProjects(): Observable<{ data: Project[] }> {
        return this.http.get<{ data: Project[] }>(`${environment.baseUrl}/projects`);
    }

    getProject(id: number): Observable<{ data: Project }> {
        return this.http.get<{ data: Project }>(`${environment.baseUrl}/projects/${id}`);
    }

    collectSOL(id: number): Observable<{ message: string }> {
        return this.http.post<{ message: string }>(`${environment.baseUrl}/projects/${id}/collect/sol`, {});
    }

    collectTokens(id: number): Observable<{ message: string }> {
        return this.http.post<{ message: string }>(`${environment.baseUrl}/projects/${id}/collect/tokens`, {});
    }

    disperseSOL(id: number): Observable<{ message: string }> {
        return this.http.post<{ message: string }>(`${environment.baseUrl}/projects/${id}/disperse/sol`, {});
    }

    disperseTokens(id: number): Observable<{ message: string }> {
        return this.http.post<{ message: string }>(`${environment.baseUrl}/projects/${id}/disperse/tokens`, {});
    }

    updateProject(id: number, updatePayload: UpdateProject): Observable<{ data: Project, message: string }> {
        return this.http.put<{ data: Project, message: string }>(`${environment.baseUrl}/projects/${id}`, updatePayload);
    }

    getProjectWallets(id: number, refresh = false): Observable<{ data: ProjectWalletsFinancials }> {
        const params = refresh ? '?refresh=true' : '';
        return this.http.get<{ data: ProjectWalletsFinancials }>(`${environment.baseUrl}/projects/${id}/wallets${params}`);
    }

    getProjectFinancials(id: number): Observable<{ data: CommonPool | null }> {
        return this.http.get<{ data: CommonPool | null }>(`${environment.baseUrl}/projects/${id}/financials`);
    }

    getProjectTransactions(id: number, page: number, limit: number): Observable<{ data: ProjectTransactions }> {
        return this.http.get<{ data: ProjectTransactions }>(`${environment.baseUrl}/projects/${id}/transactions?page=${page}&limit=${limit}`);
    }

    getProjectStatistics(id: number, limit: string): Observable<{ data: ProjectStatistics }> {
        return this.http.get<{ data: ProjectStatistics }>(`${environment.baseUrl}/projects/${id}/statistics?limit=${limit}`);
    }

    getProjectTheoPrice(id: number): Observable<{ data: string }> {
        return this.http.get<{ data: string }>(`${environment.baseUrl}/projects/${id}/theo_price`);
    }

    getProjectCurrentPosition(id: number): Observable<{ data: string }> {
        return this.http.get<{ data: string }>(`${environment.baseUrl}/projects/${id}/current_position`);
    }

    deleteProject(id: number): Observable<{ message: string }> {
        return this.http.delete<{ message: string }>(`${environment.baseUrl}/projects/${id}`);
    }

    getPools(token: string, network: string): Observable<{ data: CommonPool[] | null }> {
        return this.http.get<{ data: CommonPool[] | null }>(`${environment.baseUrl}/tokens/${token}?network=${network}`);
    }

    executeWallet(project_id: number, wallet_id: number): Observable<Project> {
        return this.http.post<Project>(`${environment.baseUrl}/projects/${project_id}/wallets/${wallet_id}/execute`, {});
    }

    startTask(project_id: number): Observable<{ message: string }> {
        return this.http.post<{ message: string }>(`${environment.baseUrl}/projects/${project_id}/start`, {});
    }

    stopTask(project_id: number): Observable<{ message: string }> {
        return this.http.post<{ message: string }>(`${environment.baseUrl}/projects/${project_id}/stop`, {});
    }

    /**
     * Users
     */
    public addUser(payload: AddUserPayload): Observable<{ message: string, data: User }> {
        return this.http.post<{ message: string, data: User }>(`${this.defaultController}/users`, payload);
    }

    public getUsers(): Observable<{ data: User[] }> {
        return this.http.get<{ data: User[] }>(`${this.defaultController}/users`);
    }

    public whitelistUser(userId: number): Observable<{ message: string }> {
        return this.http.post<{ message: string }>(`${this.defaultController}/users/${userId}/whitelist`, {});
    }

    getSystemStatus(): Observable<SystemStatus> {
        return this.http.get<SystemStatus>(`${environment.baseUrl}/system/status`);
    }

    ping(): Observable<{ version: string }> {
        return this.http.get<{ version: string }>(`${environment.baseUrl}/ping`);
    }

    checkFrontendVersion(): Observable<{ version: string }> {
        return this.http.get<{ version: string }>(`/assets/version.json?t=${Date.now()}`);
    }

    /**
     * Project Killswitch (Admin)
     */
    lockProject(projectId: number, lockAt?: number): Observable<{ message: string }> {
        const body = lockAt ? { lock_at: lockAt } : {};
        return this.http.post<{ message: string }>(`${this.defaultController}/admin/projects/${projectId}/lock`, body);
    }

    unlockProject(projectId: number): Observable<{ message: string }> {
        return this.http.delete<{ message: string }>(`${this.defaultController}/admin/projects/${projectId}/lock`);
    }

    /**
     * Project Access (Admin)
     */
    getAllProjects(): Observable<{ data: Project[] }> {
        return this.http.get<{ data: Project[] }>(`${environment.baseUrl}/projects/all`);
    }

    grantProjectAccess(userId: number, projectId: number): Observable<{ data: ProjectAccess, message: string }> {
        return this.http.post<{ data: ProjectAccess, message: string }>(`${this.defaultController}/users/${userId}/projects/${projectId}/access`, {});
    }

    revokeProjectAccess(userId: number, projectId: number): Observable<{ message: string }> {
        return this.http.delete<{ message: string }>(`${this.defaultController}/users/${userId}/projects/${projectId}/access`);
    }

    getProjectAccess(projectId: number): Observable<{ data: ProjectAccess[] }> {
        return this.http.get<{ data: ProjectAccess[] }>(`${this.defaultController}/projects/${projectId}/access`);
    }

    /**
     * Audit Logs (Admin)
     */
    getAuditLogs(userId: number | null, projectId: number | null, limit: number, offset: number): Observable<{ data: AuditLog[] }> {
        let params = `?limit=${limit}&offset=${offset}`;
        if (userId !== null) params += `&user_id=${userId}`;
        if (projectId !== null) params += `&project_id=${projectId}`;
        return this.http.get<{ data: AuditLog[] }>(`${this.defaultController}/audit${params}`);
    }

    /**
     * Subscriptions (Admin)
     */
    createSubscription(projectId: number, payload: CreateSubscriptionPayload): Observable<{ data: Subscription }> {
        return this.http.post<{ data: Subscription }>(`${this.defaultController}/admin/projects/${projectId}/subscription`, payload);
    }

    updateSubscription(projectId: number, payload: UpdateSubscriptionPayload): Observable<{ data: Subscription }> {
        return this.http.patch<{ data: Subscription }>(`${this.defaultController}/admin/projects/${projectId}/subscription`, payload);
    }

    deleteSubscription(projectId: number): Observable<{ message: string }> {
        return this.http.delete<{ message: string }>(`${this.defaultController}/admin/projects/${projectId}/subscription`);
    }

    getAllSubscriptions(): Observable<{ data: SubscriptionWithProject[] }> {
        return this.http.get<{ data: SubscriptionWithProject[] }>(`${this.defaultController}/admin/subscriptions`);
    }

    getOverdueSubscriptions(): Observable<{ data: SubscriptionWithProject[] }> {
        return this.http.get<{ data: SubscriptionWithProject[] }>(`${this.defaultController}/admin/subscriptions/overdue`);
    }

    createPayment(subscriptionId: number, payload: CreatePaymentPayload): Observable<{ data: Payment }> {
        return this.http.post<{ data: Payment }>(`${this.defaultController}/admin/subscriptions/${subscriptionId}/payments`, payload);
    }

    getPayments(subscriptionId: number): Observable<{ data: { payments: Payment[], total_paid: string } }> {
        return this.http.get<{ data: { payments: Payment[], total_paid: string } }>(`${this.defaultController}/admin/subscriptions/${subscriptionId}/payments`);
    }

    deletePayment(subscriptionId: number, paymentId: number): Observable<{ message: string }> {
        return this.http.delete<{ message: string }>(`${this.defaultController}/admin/subscriptions/${subscriptionId}/payments/${paymentId}`);
    }

    /**
     * Subscriptions (User)
     */
    getProjectSubscription(projectId: number): Observable<{ data: SubscriptionInfo | null }> {
        return this.http.get<{ data: SubscriptionInfo | null }>(`${this.defaultController}/projects/${projectId}/subscription`);
    }

    getMySubscriptions(): Observable<{ data: Record<string, SubscriptionInfo> }> {
        return this.http.get<{ data: Record<string, SubscriptionInfo> }>(`${this.defaultController}/subscriptions`);
    }

    getWorkers(): Observable<any> {
        return this.http.get(`${this.defaultController}/admin/workers`);
    }

}
