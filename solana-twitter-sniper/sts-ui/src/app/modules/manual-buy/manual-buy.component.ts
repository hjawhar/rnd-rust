import { AfterViewInit, Component } from '@angular/core';
import { FormGroup, FormControl, Validators } from '@angular/forms';
import { BuyRequestPayload } from '../../shared/models/solana_reqs.model';
import { MatSnackBar } from '@angular/material/snack-bar';
import { ApiService } from '../../shared/services/api.service';
import { CommonModule } from '@angular/common';
import { SharedModule } from '../../shared/shared.module';
import { takeUntil } from 'rxjs/internal/operators/takeUntil';
import { BasePageComponent } from '../../core/base-page.component';
import { forkJoin } from 'rxjs';
import { BlockLeader } from '../../shared/models/block_leader.model';
import { Wallet } from '../../shared/models/wallet.model';
import { Server } from '../../shared/models/server.model';

@Component({
  selector: 'app-manual-buy',
  imports: [CommonModule, SharedModule],
  templateUrl: './manual-buy.component.html',
  styleUrl: './manual-buy.component.scss'
})
export class ManualBuyComponent extends BasePageComponent implements AfterViewInit {
  taskConfiguration = new FormGroup({
    wallet_id: new FormControl<number | null>(null, [Validators.required]),
    mint_address: new FormControl<string | null>(null, [Validators.required]),
    servers: new FormControl<string[]>([], [Validators.required]),
    block_leaders: new FormControl<string[]>([], [Validators.required]),
    value: new FormControl<number>(0.01, [Validators.required]),
    tip: new FormControl<number>(0.002, [Validators.required]),
    slippage: new FormControl<number>(50, [Validators.required, Validators.min(1)]),
    tries: new FormControl<number>(1, [Validators.required, Validators.min(1)]),
    frontrunning_protection: new FormControl(true, [Validators.required]),
    enable_alerts: new FormControl(true, [Validators.required])
  });

  wallets: Wallet[] = [];
  servers: Server[] = [];
  block_leaders: BlockLeader[] = [];

  constructor(private apiService: ApiService, private snackbar: MatSnackBar) {
    super();
  }

  ngAfterViewInit(): void {
    forkJoin([
      this.apiService.getServers(),
      this.apiService.getBlockLeaders(),
      this.apiService.getWallets()
    ]).subscribe(responses => {
      this.servers = responses[0].data;
      this.block_leaders = responses[1].data;
      this.wallets = responses[2].data;
    });
  }

  public triggerBuy() {
    let { mint_address, wallet_id, value, tip, slippage, tries, frontrunning_protection, enable_alerts, servers, block_leaders } = this.taskConfiguration.getRawValue();
    let payload: BuyRequestPayload = {
      value: +value!,
      tip: +tip!,
      slippage: slippage ? +slippage : 50,
      tries: tries ? +tries : 1,
      frontrunning_protection: frontrunning_protection ? frontrunning_protection : false,
      mint_address: mint_address!,
      wallet_id: wallet_id!,
      enable_alerts: enable_alerts ? enable_alerts : false,
      servers: servers!.join(','),
      block_leaders: block_leaders!.join(','),
      selected_pool: 'ALL'
    }
    this.apiService.buy(payload).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.snackbar.open(response.data, 'Dismiss', { duration: 10000 })
    });
  }
}
