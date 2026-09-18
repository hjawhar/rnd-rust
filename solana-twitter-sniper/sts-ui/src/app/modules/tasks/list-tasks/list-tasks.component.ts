import { CommonModule } from '@angular/common';
import { AfterViewInit, Component, ViewChild } from '@angular/core';
import { MatSnackBar } from '@angular/material/snack-bar';
import { takeUntil } from 'rxjs';
import { BasePageComponent } from '../../../core/base-page.component';
import { TaskLogs } from '../../../shared/models/solana_reqs.model';
import { TwitterTweet } from '../../../shared/models/twitter.model';
import { ApiService } from '../../../shared/services/api.service';
import { WatcherService } from '../../../shared/services/watcher.service';
import { SharedModule } from '../../../shared/shared.module';
import { addressAbrev } from '../../../shared/utils';
import { Router, RouterModule } from '@angular/router';
import { Task } from '../../../shared/models/task.model';
import { Pool } from '../../../shared/models/pool.model';
import { MatPaginator } from '@angular/material/paginator';
import { MatSort } from '@angular/material/sort';
import { MatTableDataSource } from '@angular/material/table';

@Component({
  selector: 'app-list-tasks',
  imports: [CommonModule, SharedModule, RouterModule],
  templateUrl: './list-tasks.component.html',
  styleUrl: './list-tasks.component.scss'
})
export class ListTasksComponent extends BasePageComponent implements AfterViewInit {

  addressAbrev = addressAbrev;

  // tasks: Task[] = [];
  tweets: { server: string, tweet: { tweet: TwitterTweet, timestamp: number, slot: number } }[] = [];
  logs: { server: string, logs: TaskLogs }[] = [];
  pools: Pool[] = [];

  displayedColumns: string[] = ['twitter_handle', 'twitter_handle_checker', 'values', 'servers', 'block_leaders', 'selected_pool', 'actions'];
  dataSource: MatTableDataSource<Task>;

  @ViewChild(MatPaginator) paginator!: MatPaginator;
  @ViewChild(MatSort) sort!: MatSort;
  constructor(private apiService: ApiService, private snackbar: MatSnackBar, private watcherService: WatcherService, private router: Router) {
    super();
    this.dataSource = new MatTableDataSource<Task>([]);
  }

  ngAfterViewInit(): void {
    this.dataSource.paginator = this.paginator;
    this.dataSource.sort = this.sort;
    this.apiService.getPools().subscribe(response => {
      this.pools = response.data;
    });
    this.getTasks();
  }

  private getTasks() {
    this.apiService.getTasks().pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.dataSource = new MatTableDataSource(response.data);
      this.dataSource.paginator = this.paginator;
      this.dataSource.sort = this.sort;
    })
  }

  public syncTasks() {
    this.apiService.syncTasks().pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
      this.dataSource = new MatTableDataSource(response.data);
      this.dataSource.paginator = this.paginator;
      this.dataSource.sort = this.sort;
    });
  }

  public editTask(id: number) {
    this.router.navigate([`/tasks/${id}`]);
  }

  public deleteTask(id: number) {
    this.apiService.deleteTask(id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
      let data = this.dataSource.data;
      let idx = data.findIndex(x => x.id === id);
      if (idx >= 0) {
        data.splice(idx, 1);
      }
      this.dataSource = new MatTableDataSource(data);
      this.dataSource.paginator = this.paginator;
      this.dataSource.sort = this.sort;
    });
  }

  public addTask() {
    this.apiService.addTask().pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
      let data = this.dataSource.data;
      data.push(response.data);

      this.dataSource = new MatTableDataSource(data);
      this.dataSource.paginator = this.paginator;
      this.dataSource.sort = this.sort;
    });
  }

  public getPoolName(selected_pool: string) {
    let found = this.pools.find(pool => selected_pool === pool.id);
    return found ? found.name : '-'
  }

  public splitValues(input: string | null) {
    if (!input) {
      return '-';
    }
    return input.split(',').join(' ');
  }

  applyFilter(event: Event) {
    const filterValue = (event.target as HTMLInputElement).value;
    this.dataSource.filter = filterValue.trim().toLowerCase();

    if (this.dataSource.paginator) {
      this.dataSource.paginator.firstPage();
    }
  }
}
