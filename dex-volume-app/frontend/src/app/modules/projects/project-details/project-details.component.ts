import { CommonModule, DecimalPipe } from '@angular/common';
import { AfterViewInit, ChangeDetectorRef, Component, ViewChild } from '@angular/core';
import { RouterModule, ActivatedRoute, Router } from '@angular/router';
import { DomSanitizer, SafeResourceUrl } from '@angular/platform-browser';
import { SharedModule } from '../../../shared/shared.module';
import { MatSnackBar } from '@angular/material/snack-bar';
import { ApiService } from '../../../shared/services/api.service';
import { BasePageComponent } from '../../../core/base-page.component';
import { debounceTime, forkJoin, takeUntil } from 'rxjs';
import { ProjectWalletInfo, ProjectWalletSummary } from '../../../shared/models/wallets.model';
import { ListWalletsComponent } from "./components/list-wallets/list-wallets.component";
import { ListTradingLogsComponent } from "./components/list-trading-logs/list-trading-logs.component";
import { Transaction } from '../../../shared/models/transaction.model';
import { WatcherService } from '../../../shared/services/watcher.service';
import { BalanceType } from '../../../shared/models/balance_update.model';
import { addressAbrev, deepClone, isAdmin, isEmptyOrNull } from '../../../shared/utils';
import { Project, ProjectStatistics, UpdateProject } from '../../../shared/models/project.model';
import { SubscriptionInfo } from '../../../shared/models/subscription.model';
import { PageEvent } from '@angular/material/paginator';
import { ConfirmDeleteProjectComponent } from '../list-projects/dialogs/confirm-delete-project/confirm-delete-project.component';
import { MatDialog } from '@angular/material/dialog';
import { ConfirmActionComponent } from '../../../shared/components/confirm-action/confirm-action.component';
import { LockProjectComponent, LockProjectResult } from './dialogs/lock-project/lock-project.component';
import { FormControl, FormGroup, Validators } from '@angular/forms';
import { TRAGING_STRATEGY_METHODS } from '../../../shared/data/constants';
import { ThemeService } from '../../../shared/services/theme.service';
import { CommonPool } from '../../../shared/models/token.model';

enum ProjectTab {
  WALLETS,
  TRADING_LOGS
}

@Component({
  selector: 'app-project-details',
  imports: [CommonModule, RouterModule, SharedModule, ListWalletsComponent, ListTradingLogsComponent],
  templateUrl: './project-details.component.html',
  styleUrl: './project-details.component.scss'
})
export class ProjectDetailsComponent extends BasePageComponent implements AfterViewInit {
  ProjectTab = ProjectTab;
  addressAbrev = addressAbrev;
  project: Project | undefined;
  projectStatistics: ProjectStatistics | undefined;
  projectFinancials: CommonPool | undefined;
  wallets: ProjectWalletInfo[] = [];
  wallet_main: ProjectWalletInfo | undefined;
  transactions: Transaction[] = [];
  pageSize = 25;
  pageNumber = 1;
  totalCount = 0;

  volumeSettingsForm = new FormGroup({
    trading_daily_volume: new FormControl<string | null>(null),
    trading_interval: new FormControl<number | null>(null),
    max_market_impact_bps: new FormControl<number | null>(null),
    trade_multiplier: new FormControl<number | null>(null),
    slippage: new FormControl<number | null>(null),
    bundle_enabled: new FormControl<boolean>(false),
    jito_tip: new FormControl<number | null>(null)
  });

  currentTab: ProjectTab = ProjectTab.TRADING_LOGS;
  trading_mm_strategy_methods = TRAGING_STRATEGY_METHODS;
  statisticsInterval: string = '24hr';
  chartUrl: SafeResourceUrl | null = null;
  walletsSummary: ProjectWalletSummary | undefined;
  subscriptionInfo: SubscriptionInfo | undefined;
  isAdmin = isAdmin;

  @ViewChild('walletsList') walletsList!: ListWalletsComponent;

  constructor(
    private decimalPipe: DecimalPipe,
    private apiService: ApiService,
    private snackbar: MatSnackBar,
    private route: ActivatedRoute,
    private watcherService: WatcherService,
    private dialog: MatDialog,
    private router: Router,
    private cdr: ChangeDetectorRef,
    private sanitizer: DomSanitizer,
    public themeService: ThemeService
  ) {
    super();

    // Subscribe to theme changes to update chart
    this.themeService.theme$.pipe(takeUntil(this.componentDestroyed$)).subscribe(() => {
      this.updateChartUrl();
    });
  }

  private updateChartUrl(): void {
    if (this.project?.pool) {
      const theme = this.themeService.currentTheme;
      const chain = this.project.network === 'base' ? 'base' : 'solana';
      const url = `https://dexscreener.com/${chain}/${this.project.pool}?embed=1&theme=${theme}&trades=0&info=0`;
      this.chartUrl = this.sanitizer.bypassSecurityTrustResourceUrl(url);
    }
  }

  ngAfterViewInit(): void {
    const { id } = this.route.snapshot.params;
    if (!isNaN(+id)) {
      this.getProjectDetails(+id);
    }

    this.watcherService.$walletsFinancialsInfo.subscribe(response => {
      if (this.project && response && response.data && response.data.ResponseWalletsFinancials && response.data.ResponseWalletsFinancials.project_id == this.project.id) {
        let w = response.data.ResponseWalletsFinancials;
        let mainWallet = w.wallets.find(x => x.main);
        let otherWallets = w.wallets.filter(x => !x.main);
        this.wallets = otherWallets;
        this.wallet_main = mainWallet;
        this.cdr.detectChanges();
        this.walletsSummary = response.data.ResponseWalletsFinancials.summary;
        this.cdr.detectChanges();
      }
    });

    this.watcherService.$newTx.subscribe(response => {
      if (this.project && response.NewTransactionResponse && this.project.id == response.NewTransactionResponse.project_id) {
        let tx = response.NewTransactionResponse;
        this.transactions = [tx, ...this.transactions].slice(0, this.pageSize);
        this.totalCount++;

        if (this.projectStatistics && tx.slot > 0) {
          if (tx.tx_type === 'BUY') {
            this.projectStatistics.buy.native = +this.projectStatistics.buy.native + +tx.value;
            this.projectStatistics.buy.tokens = +this.projectStatistics.buy.tokens + +tx.tokens;
            this.projectStatistics.buy.usdc = +this.projectStatistics.buy.usdc + +(tx.value * tx.sol_price);
          } else if (tx.tx_type === 'SELL') {
            this.projectStatistics.sell.native = +this.projectStatistics.sell.native + +tx.value;
            this.projectStatistics.sell.tokens = +this.projectStatistics.sell.tokens + +tx.tokens;
            this.projectStatistics.sell.usdc = +this.projectStatistics.sell.usdc + +(tx.value * tx.sol_price);
          }
          this.projectStatistics.total.usdc = +this.projectStatistics.total.usdc + +(tx.value * tx.sol_price);
          this.projectStatistics.total.native = +this.projectStatistics.total.native + +tx.value;
        }
        this.cdr.detectChanges();
      }
    });

    this.watcherService.$updateTx.subscribe(response => {
      if (this.project && response.UpdateTransactionResponse && this.project.id == response.UpdateTransactionResponse.project_id) {
        let tx = response.UpdateTransactionResponse;
        let txs = deepClone(this.transactions);
        let idx = txs.findIndex(x => x.tx_hash === tx.tx_hash && x.tx_type === tx.tx_type);
        if (idx >= 0) {
          let wasUnconfirmed = txs[idx].slot === 0;
          txs[idx] = tx;
          this.transactions = txs;

          // Only update statistics when a previously-unconfirmed tx gets its real slot
          // (avoids double-counting txs that were already counted on NEW_TX)
          if (wasUnconfirmed && this.projectStatistics && tx.slot > 0) {
            if (tx.tx_type === 'BUY') {
              this.projectStatistics.buy.native = +this.projectStatistics.buy.native + +tx.value;
              this.projectStatistics.buy.tokens = +this.projectStatistics.buy.tokens + +tx.tokens;
              this.projectStatistics.buy.usdc = +this.projectStatistics.buy.usdc + +(tx.value * tx.sol_price);
            } else if (tx.tx_type === 'SELL') {
              this.projectStatistics.sell.native = +this.projectStatistics.sell.native + +tx.value;
              this.projectStatistics.sell.tokens = +this.projectStatistics.sell.tokens + +tx.tokens;
              this.projectStatistics.sell.usdc = +this.projectStatistics.sell.usdc + +(tx.value * tx.sol_price);
            }
            this.projectStatistics.total.usdc = +this.projectStatistics.total.usdc + +(tx.value * tx.sol_price);
            this.projectStatistics.total.native = +this.projectStatistics.total.native + +tx.value;
          }
        }
      }
    });

    this.watcherService.$balanceUpdate.subscribe(response => {
      if (this.project && this.projectFinancials && response.BalanceUpdate && this.project.id === response.BalanceUpdate.relation.project_id) {
        let balanceUpdate = response.BalanceUpdate;
        if (balanceUpdate.relation.owner === this.project.address) {
          if (balanceUpdate.relation.balance_type == BalanceType.QUOTE) {
            this.projectFinancials = {
              ...this.projectFinancials,
              balance1: balanceUpdate.balance / 1e6
            }
          } else {
            this.projectFinancials = {
              ...this.projectFinancials,
              balance0: balanceUpdate.balance / (this.projectFinancials.token0 == 'So11111111111111111111111111111111111111112' ? 1e9 : 1e6)
            }
          }
          // if (balanceUpdate.relation.balance_type == BalanceType.BASE || balanceUpdate.relation.balance_type == BalanceType.QUOTE) {
          //   if (balanceUpdate.relation.mint === this.projectFinancials.token0) {
          //     console.log(`1 ${balanceUpdate}`)
          //   } if (balanceUpdate.relation.address === this.projectFinancials.token1) {
          //     console.log(`2 ${balanceUpdate}`)
          //   }
          // }
          // if (balanceUpdate.relation.balance_type == BalanceType.BASE) {
          //   if (balanceUpdate.relation.address === this.projectFinancials.token0) {
          //     this.projectFinancials = {
          //       ...this.projectFinancials,
          //       balance0: balanceUpdate.balance / 1e6
          //     }
          //   }
          // }

          // if (balanceUpdate.relation.balance_type == BalanceType.QUOTE) {
          //   if (balanceUpdate.relation.address === this.projectFinancials.token1) {
          //     this.projectFinancials = {
          //       ...this.projectFinancials,
          //       balance1: balanceUpdate.balance / (this.projectFinancials.token0 == 'So11111111111111111111111111111111111111112' ? 1e9 : 1e6)
          //     }
          //   }
          // }
        } else {
          let idx = this.wallets.findIndex(wallet => wallet.address == balanceUpdate.relation.owner);
          if (idx >= 0) {
            let wallets = deepClone(this.wallets);
            let updated = false;
            if (balanceUpdate.relation.balance_type == BalanceType.WSOL) {
              wallets[idx] = {
                ...wallets[idx],
                native_balance: balanceUpdate.balance,
              }
              updated = true;
            }
            if (balanceUpdate.relation.balance_type == BalanceType.TOKENS) {
              wallets[idx] = {
                ...wallets[idx],
                token_balance: balanceUpdate.balance / 1e6,
              }
              updated = true;
            }
            // if (balanceUpdate.relation.balance_type == BalanceType.USDC) {
            //   wallets[idx] = {
            //     ...wallets[idx],
            //     usdc: balanceUpdate.balance / 1e6,
            //   }
            //   updated = true;
            // }

            if (updated) {
              this.wallets = wallets;
            }
          } else {
            if (this.wallet_main && this.wallet_main.address == balanceUpdate.relation.owner) {
              if (balanceUpdate.relation.balance_type == BalanceType.WSOL) {
                this.wallet_main = {
                  ...this.wallet_main!,
                  native_balance: balanceUpdate.balance,
                }
              }
              if (balanceUpdate.relation.balance_type == BalanceType.TOKENS) {
                this.wallet_main = {
                  ...this.wallet_main!,
                  token_balance: balanceUpdate.balance / 1e6,
                }
              }
              // if (balanceUpdate.relation.balance_type == BalanceType.USDC) {
              //   this.wallet_main = {
              //     ...this.wallet_main!,
              //     usdc: balanceUpdate.balance / 1e6,
              //   }
              // }
            }
          }
        }
      }
    });

    this.watcherService.$dailyVolume.subscribe(response => {
      if (this.project && this.projectStatistics && response.DailyVolumeUpdate && this.project.id === response.DailyVolumeUpdate.project_id) {
        this.projectStatistics = {
          ...this.projectStatistics,
          daily_volume: response.DailyVolumeUpdate
        }
      }
    });


    this.watcherService.$projectStatus.subscribe(response => {
      if (this.project && response.TaskStatusUpdate && this.project.id === response.TaskStatusUpdate.project_id) {
        this.project.status = response.TaskStatusUpdate.status;
      }
    });
  }

  public onWalletUpdated($event: ProjectWalletInfo[]) {
    let cloned = deepClone($event);
    this.wallets = cloned;
  }

  public refreshWalletsData() {
    if (!this.project) return;
    this.apiService.getProjectWallets(this.project.id!, true).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      let w = response.data;
      let mainWallet = w.wallets.find(x => x.main);
      let otherWallets = w.wallets.filter(x => !x.main);
      this.wallets = otherWallets;
      this.wallet_main = mainWallet;
      this.walletsSummary = response.data.summary;
      this.cdr.detectChanges();
    });
  }

  getProjectDetails(id: number) {
    this.pageNumber = 1;
    this.pageSize = 25;
    this.totalCount = 0;
    this.apiService.getProject(id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.project = response.data;
      this.updateChartUrl();
      if (this.project.trading_strategy == "VOLUME_MAKER") {
        this.volumeSettingsForm.get('trading_daily_volume')?.setValue(this.decimalPipe.transform(+this.project.trading_daily_volume));
        this.volumeSettingsForm.get('trading_interval')?.setValue(+this.project.trading_interval);
        this.volumeSettingsForm.get('max_market_impact_bps')?.setValue(this.project.max_market_impact_bps ? +this.project.max_market_impact_bps : null);
        this.volumeSettingsForm.get('trade_multiplier')?.setValue(this.project.trade_multiplier);
        this.volumeSettingsForm.get('slippage')?.setValue(this.project.slippage);
        this.volumeSettingsForm.get('bundle_enabled')?.setValue(this.project.bundle_enabled ?? false);
        this.volumeSettingsForm.get('jito_tip')?.setValue(this.project.jito_tip);
      }
    });
    this.apiService.getProjectWallets(id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      let w = response.data;
      let mainWallet = w.wallets.find(x => x.main);
      let otherWallets = w.wallets.filter(x => !x.main);
      this.wallets = otherWallets;
      this.wallet_main = mainWallet;
      this.cdr.detectChanges();
      this.walletsSummary = response.data.summary;
    });
    this.apiService.getProjectFinancials(id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      if (response.data) {
        this.projectFinancials = response.data;
      }
    });
    this.apiService.getProjectStatistics(id, this.statisticsInterval).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.projectStatistics = response.data;
    });
    this.apiService.getProjectTransactions(id, this.pageNumber, this.pageSize).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.transactions = response.data.transactions;
      this.totalCount = response.data.count;
      this.pageSize = response.data.limit;
    });
    this.apiService.getProjectSubscription(id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      if (response.data) {
        this.subscriptionInfo = response.data;
      }
    });
  }

  public fetchStatistics(interval: string) {
    this.statisticsInterval = interval;
    if (this.project) {
      this.apiService.getProjectStatistics(this.project.id, interval).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
        this.projectStatistics = response.data;
      });
    }
  }

  public onChangedPage($event: PageEvent) {
    if (!this.project) {
      return;
    }
    this.pageNumber = $event.pageIndex + 1;
    this.pageSize = $event.pageSize;
    this.apiService.getProjectTransactions(this.project.id, this.pageNumber, this.pageSize).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.transactions = response.data.transactions;
      this.totalCount = response.data.count;
      this.pageSize = response.data.limit;
    });
  }

  public deleteProject() {
    if (this.project && this.wallet_main) {
      this.dialog.open(ConfirmDeleteProjectComponent, {
        panelClass: "",
        maxWidth: "540px",
        width: "100%",
      }).afterClosed().subscribe(confirmResponse => {
        if (confirmResponse) {
          forkJoin([
            this.apiService.exportWallets(this.project!.id, { ids: [this.wallet_main!.id] }).pipe(takeUntil(this.componentDestroyed$)),
            this.apiService.exportWallets(this.project!.id, { ids: this.wallets.map(x => x.id) }).pipe(takeUntil(this.componentDestroyed$))
          ]).subscribe(responses => {
            {
              const link = document.createElement("a");
              const file = new Blob([JSON.stringify(responses[0].data)], { type: 'application/json' });
              link.href = URL.createObjectURL(file);
              link.download = `main_wallet_${new Date().getTime()}.json`;
              link.click();
              URL.revokeObjectURL(link.href);
            }

            {
              const link = document.createElement("a");
              const file = new Blob([JSON.stringify(responses[1].data)], { type: 'application/json' });
              link.href = URL.createObjectURL(file);
              link.download = `wallets_${new Date().getTime()}.json`;
              link.click();
              URL.revokeObjectURL(link.href);
            }
            this.apiService.deleteProject(this.project!.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
              this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
              this.router.navigate(['/projects']);
            });
          });
        }
      });
    }
  }

  public download() {
    this.apiService.exportWallets(this.project!.id, { ids: [this.wallet_main!.id] }).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      const link = document.createElement("a");
      const file = new Blob([JSON.stringify(response.data)], { type: 'application/json' });
      link.href = URL.createObjectURL(file);
      link.download = `main_wallet_${new Date().getTime()}.json`;
      link.click();
      URL.revokeObjectURL(link.href);
    });
  }

  public collectSOL() {
    this.dialog.open(ConfirmActionComponent, {
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
      data: {
        title: `Collect ${this.nativeSymbol}`,
        action: `collect ${this.nativeSymbol}`
      }
    }).afterClosed().subscribe(confirmResponse => {
      if (confirmResponse) {
        this.apiService.collectSOL(this.project!.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
        });
      }
    });
  }

  public collectTokens() {
    this.dialog.open(ConfirmActionComponent, {
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
      data: {
        title: 'Collect Tokens',
        action: 'collect tokens'
      }
    }).afterClosed().subscribe(confirmResponse => {
      if (confirmResponse) {
        this.apiService.collectTokens(this.project!.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
        });
      }
    });
  }

  public disperseSOL() {
    this.dialog.open(ConfirmActionComponent, {
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
      data: {
        title: `Disperse ${this.nativeSymbol}`,
        action: `disperse ${this.nativeSymbol}`
      }
    }).afterClosed().subscribe(confirmResponse => {
      if (confirmResponse) {
        this.apiService.disperseSOL(this.project!.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
        });
      }
    });
  }

  public disperseTokens() {
    this.dialog.open(ConfirmActionComponent, {
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
      data: {
        title: 'Disperse Tokens',
        action: 'disperse tokens'
      }
    }).afterClosed().subscribe(confirmResponse => {
      if (confirmResponse) {
        this.apiService.disperseTokens(this.project!.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
        });
      }
    });
  }

  public startTask() {
    if (this.project) {
      this.apiService.startTask(this.project.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
        this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
      });
    }
  }

  public stopTask() {
    if (this.project) {
      this.apiService.stopTask(this.project.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
        this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
      });
    }
  }


  getTokenSymbol = (token: string) => {
    switch (token) {
      case 'So11111111111111111111111111111111111111112':
        return 'WSOL';
      case 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v':
        return 'USDC';
      default:
        return token;
    }
  }


  getTradingStrategy = (trading_strategy: string) => {
    if (trading_strategy == 'VOLUME_MAKER') {
      return 'Volume Maker';
    } else if (trading_strategy == 'MARKET_MAKER') {
      return 'Market Maker';
    } else {
      return 'Buy / Sell'
    }
  }

  public save() {
    const {
      trading_daily_volume,
      trading_interval,
      max_market_impact_bps,
      trade_multiplier,
      slippage,
      bundle_enabled,
      jito_tip
    } = this.volumeSettingsForm.getRawValue();
    let payload: UpdateProject = {
      trading_daily_volume: trading_daily_volume ? +trading_daily_volume?.replaceAll(',', '')! : null,
      trading_interval,
      max_market_impact_bps,
      trade_multiplier,
      slippage,
      bundle_enabled,
      jito_tip
    }
    this.apiService.updateProject(this.project!.id, payload).subscribe(response => {
      this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
    })
  }

  public getTradingCost() {
    const { trading_daily_volume, trading_interval } = this.volumeSettingsForm.getRawValue();
    if (!this.watcherService.solPrice || !trading_interval || !trading_daily_volume || !this.wallets.length) {
      return 'N/A';
    }
    let frequency = 86400.0 / trading_interval;
    let v = isNaN(+`${trading_daily_volume}`.replaceAll(',', '')) ? 0 : +`${trading_daily_volume}`.replaceAll(',', '');
    let value_usdc = v / frequency;
    let value_sol = value_usdc / this.watcherService.solPrice;
    return `${this.decimalPipe.transform(value_sol, '1.2-4')} ${this.nativeSymbol} / ${this.decimalPipe.transform(value_usdc, '1.2-4')} USDC per wallet`
  }

  get nativeSymbol(): string {
    return this.project?.network === 'base' ? 'ETH' : 'SOL';
  }

  get explorerBaseUrl(): string {
    return this.project?.network === 'base' ? 'https://basescan.org' : 'https://solscan.io';
  }

  get dexscreenerChain(): string {
    return this.project?.network === 'base' ? 'base' : 'solana';
  }

  public isVolumeMaker() {
    return this.project != null && this.project.trading_strategy == "VOLUME_MAKER";
  }

  public changeTab(tab: ProjectTab) {
    this.currentTab = tab;
    this.cdr.detectChanges();
  }

  public toggleLock() {
    if (!this.project) return;

    if (this.project.locked) {
      // Unlock — simple confirmation
      this.dialog.open(ConfirmActionComponent, {
        panelClass: "",
        maxWidth: "540px",
        width: "100%",
        data: { title: 'Unlock Project', action: 'unlock this project' }
      }).afterClosed().subscribe(confirmed => {
        if (confirmed) {
          this.apiService.unlockProject(this.project!.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
            this.project!.locked = false;
            this.project!.lock_at = null;
            this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
          });
        }
      });
    } else {
      // Lock — show dialog with immediate/scheduled options
      this.dialog.open(LockProjectComponent, {
        panelClass: "",
        maxWidth: "540px",
        width: "100%",
      }).afterClosed().subscribe((result: LockProjectResult | undefined) => {
        if (!result) return;
        if (result.mode === 'immediate') {
          this.apiService.lockProject(this.project!.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
            this.project!.locked = true;
            this.project!.lock_at = null;
            this.project!.status = 'stopped';
            this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
          });
        } else {
          this.apiService.lockProject(this.project!.id, result.lockAt).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
            this.project!.lock_at = { secs_since_epoch: result.lockAt, nanos_since_epoch: 0 };
            this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
          });
        }
      });
    }
  }

  public cancelScheduledLock() {
    if (!this.project) return;
    this.apiService.unlockProject(this.project.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.project!.lock_at = null;
      this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
    });
  }
}
