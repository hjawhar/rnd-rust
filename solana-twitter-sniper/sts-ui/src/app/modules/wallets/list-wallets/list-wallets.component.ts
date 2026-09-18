import { AfterViewInit, Component } from '@angular/core';
import { MAT_FORM_FIELD_DEFAULT_OPTIONS, MatFormFieldModule } from '@angular/material/form-field';
import { MatDialog } from '@angular/material/dialog';
import { takeUntil } from 'rxjs/operators';
import { BasePageComponent } from '../../../core/base-page.component';
import { ApiService } from '../../../shared/services/api.service';
import { addressAbrev, deepClone } from '../../../shared/utils';
import { Wallet } from '../../../shared/models/wallet.model';
import { CommonModule } from '@angular/common';
import { SharedModule } from '../../../shared/shared.module';
import { MatInputModule } from '@angular/material/input';
import { MatTableDataSource, MatTableModule } from '@angular/material/table';
import { GenerateWalletsComponent } from '../dialogs/generate-wallets/generate-wallets.component';
import { SelectionModel } from '@angular/cdk/collections';
import { MatSnackBar } from '@angular/material/snack-bar';
import { ImportWalletsComponent } from '../dialogs/import-wallets/import-wallets.component';
import { ConfirmDeleteWalletsComponent } from '../dialogs/confirm-delete-wallets/confirm-delete-wallets.component';

@Component({
  selector: 'app-list-wallets',
  templateUrl: './list-wallets.component.html',
  styleUrls: ['./list-wallets.component.scss'],
  imports: [CommonModule, SharedModule, MatFormFieldModule, MatInputModule, MatTableModule],
  providers: [
    { provide: MAT_FORM_FIELD_DEFAULT_OPTIONS, useValue: { subscriptSizing: 'dynamic' } }
  ]
})
export class ListWalletsComponent extends BasePageComponent implements AfterViewInit {
  addressAbrev = addressAbrev;
  displayedColumns: string[] = ['select', 'id', 'address', 'nonce_account_address', 'name', 'comments', 'balance'];
  dataSource = new MatTableDataSource<Wallet>([]);

  initialSelection = [];
  allowMultiSelect = true;
  selection = new SelectionModel<number>(this.allowMultiSelect, this.initialSelection);

  applyFilter(event: Event) {
    const filterValue = (event.target as HTMLInputElement).value;
    this.dataSource.filter = filterValue.trim().toLowerCase();
  }

  intervalId: any;
  constructor(public dialog: MatDialog, private apiService: ApiService, private snackbar: MatSnackBar) {
    super();
  }

  ngAfterViewInit(): void {
    this.getWallets();
  }

  override ngOnDestroy(): void {
    if (this.intervalId) {
      clearInterval(this.intervalId);
    }
  }

  private getWallets() {
    this.apiService.getWallets().pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.dataSource.data = response.data;
    });
  }

  generateWallets() {
    this.dialog.open(GenerateWalletsComponent, {
      backdropClass: "popup-backdrop",
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
    }).afterClosed().subscribe(response => {
      if (response) {
        this.apiService.generateWallets(response).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          let wallets = [...response.data, ...this.dataSource.data];
          this.dataSource.data = wallets;
        });
      }
    })
  }

  importWallets() {
    this.dialog.open(ImportWalletsComponent, {
      backdropClass: "popup-backdrop",
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
    }).afterClosed().subscribe(response => {
      if (response) {
        this.apiService.importWallets(response).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          let wallets = [...response.data, ...this.dataSource.data];
          this.dataSource.data = wallets;
        });
      }
    })
  }

  exportWallets() {
    if (!this.selection.hasValue()) {
      this.snackbar.open('Please select wallet(s) to export', 'Dismiss', { duration: 10000 });
      return;
    }
    let payload = {
      ids: this.selection.selected
    }
    this.apiService.exportWallets(payload).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      const link = document.createElement("a");
      const file = new Blob([JSON.stringify(response.data)], { type: 'application/json' });
      link.href = URL.createObjectURL(file);
      link.download = `wallets_${new Date().getTime()}.json`;
      link.click();
      URL.revokeObjectURL(link.href);
    });
  }

  deleteWallets() {
    if (!this.selection.hasValue()) {
      this.snackbar.open('Please select wallet(s) to export', 'Dismiss', { duration: 10000 });
      return;
    }
    let payload = {
      ids: this.selection.selected
    }

    this.dialog.open(ConfirmDeleteWalletsComponent, {
      backdropClass: "popup-backdrop",
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
    }).afterClosed().subscribe(confirmResponse => {
      if (confirmResponse) {
        this.apiService.deleteWallets(payload).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
          let wallets = this.dataSource.data.filter(w => this.selection.selected.indexOf(w.id) === -1);
          this.dataSource.data = wallets;
        });
      }
    });
  }

  /** Whether the number of selected elements matches the total number of rows. */
  isAllSelected() {
    const numSelected = this.selection.selected.length;
    const numRows = this.dataSource.data.length;
    return numSelected == numRows;
  }

  /** Selects all rows if they are not all selected; otherwise clear selection. */
  toggleAllRows() {
    this.isAllSelected() ?
      this.selection.clear() :
      this.dataSource.data.forEach(row => this.selection.select(row.id));
  }

  public createNonceAccount(input: Wallet) {
    this.apiService.createNonceAccount(input.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
      return;
    })
  }
}