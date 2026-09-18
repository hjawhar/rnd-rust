import { AfterViewInit, ChangeDetectorRef, Component, EventEmitter, Input, OnChanges, Output, SimpleChanges, ViewChild } from '@angular/core';
import { MatPaginator } from '@angular/material/paginator';
import { MatTableDataSource } from '@angular/material/table';
import { ProjectWalletInfo } from '../../../../../shared/models/wallets.model';
import { CommonModule } from '@angular/common';
import { RouterModule } from '@angular/router';
import { SharedModule } from '../../../../../shared/shared.module';
import { ImportWalletsComponent } from '../../dialogs/import-wallets/import-wallets.component';
import { takeUntil } from 'rxjs';
import { GenerateWalletsComponent } from '../../dialogs/generate-wallets/generate-wallets.component';
import { BasePageComponent } from '../../../../../core/base-page.component';
import { ApiService } from '../../../../../shared/services/api.service';
import { MatDialog } from '@angular/material/dialog';
import { SelectionModel } from '@angular/cdk/collections';
import { MatSnackBar } from '@angular/material/snack-bar';
import { ConfirmDeleteWalletsComponent } from '../../dialogs/confirm-delete-wallets/confirm-delete-wallets.component';

@Component({
  selector: 'app-list-wallets',
  imports: [CommonModule, RouterModule, SharedModule],
  templateUrl: './list-wallets.component.html',
  styleUrl: './list-wallets.component.scss'
})
export class ListWalletsComponent extends BasePageComponent implements AfterViewInit, OnChanges {
  displayedColumns: string[] = ['select', 'wallet', 'token_balance', 'native_balance', 'total_usdc'];
  dataSource: MatTableDataSource<ProjectWalletInfo> = new MatTableDataSource();;

  allowMultiSelect = true;
  selectionWallets = new SelectionModel<number>(this.allowMultiSelect, []);

  @ViewChild(MatPaginator) paginator!: MatPaginator;

  @Input() projectId!: number;
  @Input() wallets: ProjectWalletInfo[] = [];

  @Output() walletUpdated = new EventEmitter<ProjectWalletInfo[]>();
  @Input() nativeSymbol = 'SOL';
  @Input() explorerBaseUrl = 'https://solscan.io';
  @Input() displayFlagBuy = false;
  @Input() displayFlagSell = false;
  @Input() displaySellFields = false;

  @Output() onCheckedWallet = new EventEmitter<{ id: number, checked: boolean }>();
  @Output() onCheckedWallets = new EventEmitter<boolean>();
  @Output() onSellWallets = new EventEmitter<{ ids: number[], value: number }>();

  @Output() refreshWallets = new EventEmitter<boolean>();
  constructor(private apiService: ApiService, private dialog: MatDialog, private snackbar: MatSnackBar, private cdr: ChangeDetectorRef) {
    super();
  }

  ngAfterViewInit(): void {
    this.dataSource.paginator = this.paginator;
    if (this.displayFlagBuy && this.displayFlagSell) {
      this.displayedColumns = ['select', 'flag_buy', 'flag_sell', 'wallet', 'token_balance', 'usdc_balance', 'sol_balance', 'status'];
      this.cdr.detectChanges();
    }
    if (this.displaySellFields) {
      this.displayedColumns = ['select', 'sell_fields', 'wallet', 'token_balance', 'usdc_balance', 'sol_balance', 'status'];
      this.cdr.detectChanges();
    }
  }

  ngOnChanges(changes: SimpleChanges): void {
    if (changes && changes['wallets'] && changes['wallets'].currentValue) {
      this.initWallets(changes['wallets'].currentValue);
    }
  }

  applyFilter(event: Event) {
    const filterValue = (event.target as HTMLInputElement).value;
    this.dataSource.filter = filterValue.trim().toLowerCase();

    if (this.dataSource.paginator) {
      this.dataSource.paginator.firstPage();
    }
  }

  initWallets(wallets: ProjectWalletInfo[]) {
    this.dataSource = new MatTableDataSource(wallets);
    this.dataSource.paginator = this.paginator;
  }

  isAllSelectedWallets() {
    const numSelected = this.selectionWallets.selected.length;
    const numRows = this.dataSource.data.length;
    return numSelected == numRows;
  }

  toggleAllRowsWallets($event: Event) {
    this.isAllSelectedWallets() ? this.selectionWallets.clear() : this.dataSource.data.forEach(row => this.selectionWallets.select(row.id));
    this.onCheckedWallets.emit(($event.target as any).checked)
  }

  generateWallets() {
    this.dialog.open(GenerateWalletsComponent, {
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
    }).afterClosed().subscribe(response => {
      if (response) {
        this.apiService.generateWallets(this.projectId, response).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          // let wallets = [...response.data, ...this.dataSource.data];
          // this.walletUpdated.emit(wallets);
          // this.dataSource.data = wallets; 
          this.refreshWallets.emit(true);
        });
      }
    })
  }

  importWallets() {
    this.dialog.open(ImportWalletsComponent, {
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
    }).afterClosed().subscribe(response => {
      if (response) {
        this.apiService.importWallets(this.projectId, response).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          // let wallets = [...response.data, ...this.dataSource.data];
          // this.walletUpdated.emit(wallets);
          // this.dataSource.data = wallets;
          this.refreshWallets.emit(true);
        });
      }
    })
  }

  exportWallets() {
    if (!this.selectionWallets.hasValue()) {
      this.snackbar.open('Please select wallet(s) to export', 'Dismiss', { duration: 10000 });
      return;
    }
    let payload = {
      ids: this.selectionWallets.selected
    }
    this.apiService.exportWallets(this.projectId, payload).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      const link = document.createElement("a");
      const file = new Blob([JSON.stringify(response.data)], { type: 'application/json' });
      link.href = URL.createObjectURL(file);
      link.download = `wallets_${new Date().getTime()}.json`;
      link.click();
      URL.revokeObjectURL(link.href);
    });
  }

  deleteWallets() {
    if (!this.selectionWallets.hasValue()) {
      this.snackbar.open('Please select wallet(s) to export', 'Dismiss', { duration: 10000 });
      return;
    }
    let payload = {
      ids: this.selectionWallets.selected
    }

    this.dialog.open(ConfirmDeleteWalletsComponent, {
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
    }).afterClosed().subscribe(confirmResponse => {
      if (confirmResponse) {
        this.apiService.deleteWallets(this.projectId, payload).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          let wallets = this.dataSource.data.filter(w => this.selectionWallets.selected.indexOf(w.id) === -1);
          this.walletUpdated.emit(wallets);
          this.dataSource.data = wallets;
          this.refreshWallets.emit(true);
        });
      }
    });
  }

  public _onCheckedWallet(id: number, $event: Event) {
    this.onCheckedWallet.emit({ id, checked: ($event.target as any).checked });
  }

  public _onCheckedWallets($event: Event) {
    this.onCheckedWallets.emit(($event.target as any).checked);
  }

  public _sellWallets(ids: number[], value: number) {
    this.onSellWallets.emit({ ids, value });
  }
}
