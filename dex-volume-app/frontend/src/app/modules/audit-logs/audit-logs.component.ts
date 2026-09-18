import { CommonModule } from '@angular/common';
import { AfterViewInit, Component, ViewChild } from '@angular/core';
import { MatFormFieldModule, MAT_FORM_FIELD_DEFAULT_OPTIONS } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatTableModule, MatTableDataSource } from '@angular/material/table';
import { MatPaginator, MatPaginatorModule, PageEvent } from '@angular/material/paginator';
import { MatSort } from '@angular/material/sort';
import { MatSelectModule } from '@angular/material/select';
import { FormsModule } from '@angular/forms';
import { BasePageComponent } from '../../core/base-page.component';
import { ApiService } from '../../shared/services/api.service';
import { SharedModule } from '../../shared/shared.module';
import { AuditLog } from '../../shared/models/audit-log.model';
import { User } from '../../shared/models/user.model';
import { Project } from '../../shared/models/project.model';
import { forkJoin, takeUntil } from 'rxjs';

@Component({
  selector: 'app-audit-logs',
  templateUrl: './audit-logs.component.html',
  styleUrls: ['./audit-logs.component.scss'],
  imports: [CommonModule, SharedModule, MatFormFieldModule, MatInputModule, MatTableModule, MatSelectModule, FormsModule, MatPaginatorModule],
  providers: [
    { provide: MAT_FORM_FIELD_DEFAULT_OPTIONS, useValue: { subscriptSizing: 'dynamic' } }
  ]
})
export class AuditLogsComponent extends BasePageComponent implements AfterViewInit {
  displayedColumns: string[] = ['id', 'user_id', 'project_id', 'action', 'details', 'created_at'];
  dataSource = new MatTableDataSource<AuditLog>([]);
  loading = true;

  users: User[] = [];
  projects: Project[] = [];
  userMap = new Map<number, string>();
  projectMap = new Map<number, string>();

  filterUserId: number | null = null;
  filterProjectId: number | null = null;

  @ViewChild(MatPaginator) paginator!: MatPaginator;
  @ViewChild(MatSort) sort!: MatSort;

  constructor(private apiService: ApiService) {
    super();
  }

  ngAfterViewInit(): void {
    this.loadFilters();
    this.fetchLogs();
  }

  private loadFilters() {
    forkJoin({
      users: this.apiService.getUsers(),
      projects: this.apiService.getAllProjects(),
    }).pipe(takeUntil(this.componentDestroyed$)).subscribe(({ users, projects }) => {
      this.users = users.data;
      this.projects = projects.data;
      for (const u of this.users) {
        this.userMap.set(u.id, u.address);
      }
      for (const p of this.projects) {
        this.projectMap.set(p.id, p.name || p.symbol || p.address);
      }
    });
  }

  fetchLogs() {
    this.loading = true;
    this.apiService.getAuditLogs(this.filterUserId, this.filterProjectId, 1000, 0)
      .pipe(takeUntil(this.componentDestroyed$))
      .subscribe({
        next: (response) => {
          this.dataSource.data = response.data;
          this.dataSource.paginator = this.paginator;
          this.dataSource.sort = this.sort;
          this.loading = false;
        },
        error: () => {
          this.loading = false;
        }
      });
  }

  applyFilter(event: Event) {
    const filterValue = (event.target as HTMLInputElement).value;
    this.dataSource.filter = filterValue.trim().toLowerCase();
  }

  onFilterChange() {
    this.fetchLogs();
  }

  clearFilters() {
    this.filterUserId = null;
    this.filterProjectId = null;
    this.fetchLogs();
  }

  getUserLabel(userId: number): string {
    const addr = this.userMap.get(userId);
    if (!addr) return `#${userId}`;
    return `${addr.slice(0, 5)}...${addr.slice(-5)}`;
  }

  getProjectLabel(projectId: number | null): string {
    if (projectId === null) return '-';
    const name = this.projectMap.get(projectId);
    return name || `#${projectId}`;
  }

  formatDetails(details: Record<string, any> | null): string {
    if (!details) return '-';
    return JSON.stringify(details);
  }

  getActionLabel(action: string): string {
    const labels: Record<string, string> = {
      'project.create': 'Project Created',
      'project.update': 'Project Updated',
      'project.delete': 'Project Deleted',
      'task.start': 'Task Started',
      'task.stop': 'Task Stopped',
      'collect.native': 'Collect Native',
      'collect.tokens': 'Collect Tokens',
      'disperse.native': 'Disperse Native',
      'disperse.tokens': 'Disperse Tokens',
      'wallet.import': 'Wallets Imported',
      'wallet.generate': 'Wallets Generated',
      'wallet.delete': 'Wallets Deleted',
      'access.grant': 'Access Granted',
      'access.revoke': 'Access Revoked',
    };
    return labels[action] || action;
  }

  getActionIcon(action: string): string {
    const icons: Record<string, string> = {
      'project.create': 'bi-plus-circle',
      'project.update': 'bi-pencil',
      'project.delete': 'bi-trash',
      'task.start': 'bi-play-circle',
      'task.stop': 'bi-stop-circle',
      'collect.native': 'bi-box-arrow-in-down',
      'collect.tokens': 'bi-box-arrow-in-down',
      'disperse.native': 'bi-box-arrow-up',
      'disperse.tokens': 'bi-box-arrow-up',
      'wallet.import': 'bi-download',
      'wallet.generate': 'bi-wallet2',
      'wallet.delete': 'bi-trash',
      'access.grant': 'bi-person-plus',
      'access.revoke': 'bi-person-dash',
    };
    return icons[action] || 'bi-activity';
  }
}
