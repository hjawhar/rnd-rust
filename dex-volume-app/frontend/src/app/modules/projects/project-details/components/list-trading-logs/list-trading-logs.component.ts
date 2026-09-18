import { CommonModule } from '@angular/common';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { AfterViewInit, Component, EventEmitter, Input, OnChanges, Output, SimpleChanges, ViewChild } from '@angular/core';
import { MatDialog } from '@angular/material/dialog';
import { MatPaginator, PageEvent } from '@angular/material/paginator';
import { MatTableDataSource } from '@angular/material/table';
import { RouterModule } from '@angular/router';
import { BasePageComponent } from '../../../../../core/base-page.component';
import { ApiService } from '../../../../../shared/services/api.service';
import { SharedModule } from '../../../../../shared/shared.module';
import { Transaction } from '../../../../../shared/models/transaction.model';
import { addressAbrev, formatDate } from '../../../../../shared/utils';
import { WatcherService } from '../../../../../shared/services/watcher.service';

@Component({
  selector: 'app-list-trading-logs',
  imports: [CommonModule, RouterModule, SharedModule, MatProgressSpinnerModule],
  templateUrl: './list-trading-logs.component.html',
  styleUrl: './list-trading-logs.component.scss'
})
export class ListTradingLogsComponent extends BasePageComponent implements AfterViewInit, OnChanges {
  displayedColumns: string[] = ['time', 'slot', 'tx_hash', 'wallet', 'type', 'tokens', 'value'];
  dataSource: MatTableDataSource<Transaction> = new MatTableDataSource();;

  @ViewChild(MatPaginator) paginator!: MatPaginator;

  @Input() transactions: Transaction[] = [];
  @Input() pageSize = 25;
  @Input() totalCount = 0;
  @Input() explorerBaseUrl = 'https://solscan.io';

  @Output() onChangedPage = new EventEmitter<PageEvent>();
  addressAbrev = addressAbrev;


  constructor(private apiService: ApiService, private dialog: MatDialog, public watcherService: WatcherService) {
    super();
  }

  ngAfterViewInit(): void {
  }

  ngOnChanges(changes: SimpleChanges): void {
    if (changes && changes['transactions'] && changes['transactions'].currentValue) {
      this.initTransactions(changes['transactions'].currentValue);
    }
  }

  applyFilter(event: Event) {
    const filterValue = (event.target as HTMLInputElement).value;
    this.dataSource.filter = filterValue.trim().toLowerCase();

    if (this.dataSource.paginator) {
      this.dataSource.paginator.firstPage();
    }
  }

  initTransactions(transactions: Transaction[]) {
    this.dataSource = new MatTableDataSource(transactions);
    // this.dataSource.paginator = this.paginator;
  }

  public formatTxDate(transaction: Transaction) {
    return formatDate(new Date(transaction.date_added.secs_since_epoch * 1000));
  }
}
