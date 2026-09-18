import { Injectable } from '@angular/core';
import { Subject } from 'rxjs';
import { Transaction } from '../models/transaction.model';
import { BalanceUpdate } from '../models/balance_update.model';
import { ProjectWalletsFinancials } from '../models/wallets.model';
import { DailyVolume } from '../models/project.model';

@Injectable({
    providedIn: 'root'
})
export class WatcherService {
    // $updateTask = new Subject<{ id: number, data: Task }>();
    slotNumber: number | undefined;
    solPrice: number | undefined;
    ethPrice: number | undefined;
    $newTx = new Subject<{ NewTransactionResponse: Transaction }>();
    $updateTx = new Subject<{ UpdateTransactionResponse: Transaction }>();
    $projectStatus = new Subject<{ TaskStatusUpdate: { project_id: number, status: string } }>();
    $balanceUpdate = new Subject<{ BalanceUpdate: BalanceUpdate }>();
    $walletsFinancialsInfo = new Subject<StreamResponse<{ ResponseWalletsFinancials: ProjectWalletsFinancials }>>();
    $dailyVolume = new Subject<{ DailyVolumeUpdate: DailyVolume }>();
}


export type StreamResponse<T> = {
    id: number,
    message: number,
    type: string,
    data: T
} 