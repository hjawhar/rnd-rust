import { Injectable } from '@angular/core';
import { HttpClient } from '@angular/common/http';
import { Observable } from 'rxjs';
import { MatSnackBar } from '@angular/material/snack-bar';
import { environment } from '../../../environments/environment';
import { BuyRequestPayload } from '../models/solana_reqs.model';
import { Task, UpdateTaskPayload } from '../models/task.model';
import { Wallet, WalletFull } from '../models/wallet.model';
import { Server } from '../models/server.model';
import { BlockLeader } from '../models/block_leader.model';
import { Pool } from '../models/pool.model';
import { AddUserPayload, User } from '../models/user.model';

@Injectable({
    providedIn: 'root'
})
export class ApiService {
    protected defaultController = `${environment.baseUrl}`;
    constructor(public http: HttpClient, private snackBar: MatSnackBar) {
    }

    public getNonce(address: string): Observable<any> {
        return this.http.post(`${this.defaultController}/oauth/nonce`, { address });
    }

    /**
     * Twiiter
     */

    public getTasks(): Observable<{ data: Task[] }> {
        return this.http.get<{ data: Task[] }>(`${this.defaultController}/tasks`);
    }

    public getTask(id: number): Observable<{ data: Task }> {
        return this.http.get<{ data: Task }>(`${this.defaultController}/tasks/${id}`);
    }

    public updateTask(id: number, payload: UpdateTaskPayload): Observable<{ data: Task, message: string }> {
        return this.http.put<{ data: Task, message: string }>(`${this.defaultController}/tasks/${id}`, payload);
    }

    public syncTasks(): Observable<{ data: Task[], message: string }> {
        return this.http.post<{ data: Task[], message: string }>(`${this.defaultController}/tasks/sync`, {});
    }

    public deleteTask(id: number): Observable<{ data: number, message: string }> {
        return this.http.delete<{ data: number, message: string }>(`${this.defaultController}/tasks/${id}`);
    }

    public addTask(): Observable<{ data: Task, message: string }> {
        return this.http.post<{ data: Task, message: string }>(`${this.defaultController}/tasks`, {});
    }
    /**
     * Solana request
     */

    public buy(payload: BuyRequestPayload): Observable<{ data: string }> {
        return this.http.post<{ data: string }>(`${this.defaultController}/buy`, payload);
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
    /**
     * Wallets
     */

    public getWallets(): Observable<{ data: Wallet[] }> {
        return this.http.get<{ data: Wallet[] }>(`${this.defaultController}/wallets`);
    }
    public importWallets(payload: { private_keys: string[], name: string, comments?: string }): Observable<{ data: Wallet[] }> {
        return this.http.post<{ data: Wallet[] }>(`${this.defaultController}/wallets/import`, payload);
    }
    public exportWallets(payload: { ids: number[] }): Observable<{ data: WalletFull[] }> {
        return this.http.post<{ data: WalletFull[] }>(`${this.defaultController}/wallets/export`, payload);
    }
    public generateWallets(payload: { value: number, name: string, comments?: string }): Observable<{ data: Wallet[] }> {
        return this.http.post<{ data: Wallet[] }>(`${this.defaultController}/wallets/generate`, payload);
    }
    public deleteWallets(payload: { ids: number[] }): Observable<{ data: string }> {
        return this.http.post<{ data: string }>(`${this.defaultController}/wallets/delete`, payload);
    }
    public createNonceAccount(id: number): Observable<{ message: string }> {
        return this.http.post<{ message: string }>(`${this.defaultController}/wallets/${id}/create_nonce_account`, {});
    }

    /**
     * Blockchain
     */
    public getServers(): Observable<{ data: Server[] }> {
        return this.http.get<{ data: Server[] }>(`${this.defaultController}/servers`);
    }
    public getBlockLeaders(): Observable<{ data: BlockLeader[] }> {
        return this.http.get<{ data: BlockLeader[] }>(`${this.defaultController}/block_leaders`);
    }
    public getPools(): Observable<{ data: Pool[] }> {
        return this.http.get<{ data: Pool[] }>(`${this.defaultController}/pools`);
    }
}
