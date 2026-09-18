import { SelectionModel } from '@angular/cdk/collections';
import { CommonModule } from '@angular/common';
import { AfterViewInit, Component, ViewChild } from '@angular/core';
import { MatDialog } from '@angular/material/dialog';
import { MatFormFieldModule, MAT_FORM_FIELD_DEFAULT_OPTIONS } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatSnackBar } from '@angular/material/snack-bar';
import { MatTableModule, MatTableDataSource } from '@angular/material/table';
import { takeUntil } from 'rxjs';
import { BasePageComponent } from '../../../core/base-page.component';
import { ApiService } from '../../../shared/services/api.service';
import { SharedModule } from '../../../shared/shared.module';
import { addressAbrev } from '../../../shared/utils';
import { User } from '../../../shared/models/user.model';
import { AddUserComponent } from '../dialogs/add-user/add-user.component';
import { AssignProjectsComponent } from '../dialogs/assign-projects/assign-projects.component';
import { MatPaginator } from '@angular/material/paginator';
import { MatSort } from '@angular/material/sort';

@Component({
  selector: 'app-list-users',
  templateUrl: './list-users.component.html',
  styleUrls: ['./list-users.component.scss'],
  imports: [CommonModule, SharedModule, MatFormFieldModule, MatInputModule, MatTableModule],
  providers: [
    { provide: MAT_FORM_FIELD_DEFAULT_OPTIONS, useValue: { subscriptSizing: 'dynamic' } }
  ]
})
export class ListUsersComponent extends BasePageComponent implements AfterViewInit {
  addressAbrev = addressAbrev;
  displayedColumns: string[] = ['id', 'group_id', 'address', 'last_login_date_time', 'whitelisted', 'edit'];
  dataSource = new MatTableDataSource<User>([]);

  initialSelection = [];
  allowMultiSelect = true;
  selection = new SelectionModel<number>(this.allowMultiSelect, this.initialSelection);
  @ViewChild(MatPaginator) paginator!: MatPaginator;
  @ViewChild(MatSort) sort!: MatSort;

  applyFilter(event: Event) {
    const filterValue = (event.target as HTMLInputElement).value;
    this.dataSource.filter = filterValue.trim().toLowerCase();
  }

  intervalId: any;
  constructor(public dialog: MatDialog, private apiService: ApiService, private snackbar: MatSnackBar) {
    super();
    this.dataSource.paginator = this.paginator;
    this.dataSource.sort = this.sort;
  }

  ngAfterViewInit(): void {
    this.getUsers();
  }

  override ngOnDestroy(): void {
    if (this.intervalId) {
      clearInterval(this.intervalId);
    }
  }

  private getUsers() {
    this.apiService.getUsers().pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.dataSource.data = response.data;
      this.dataSource.paginator = this.paginator;
      this.dataSource.sort = this.sort;
    });
  }

  public editUser(userId: number) {
    this.apiService.whitelistUser(userId).subscribe(response => {
      let users = this.dataSource.data;
      let idx = users.findIndex(user => user.id === userId);
      if (idx >= 0) {
        users[idx].whitelisted = !users[idx].whitelisted;
      }

      this.dataSource.data = users;
      this.dataSource.paginator = this.paginator;
      this.dataSource.sort = this.sort;
    })
  }

  addUsers() {
    this.dialog.open(AddUserComponent, {
      backdropClass: "popup-backdrop",
      panelClass: "",
      maxWidth: "540px",
      width: "100%",
    }).afterClosed().subscribe(response => {
      if (response) {
        this.apiService.addUser(response).subscribe(response => {
          if (response.data) {
            let users = [...this.dataSource.data, response.data];
            this.dataSource.data = users;
            this.dataSource.paginator = this.paginator;
            this.dataSource.sort = this.sort;
          }
        });
      }
    })
  }

  assignProjects(userId: number) {
    this.dialog.open(AssignProjectsComponent, {
      backdropClass: "popup-backdrop",
      panelClass: "",
      maxWidth: "640px",
      width: "100%",
      data: { userId },
    });
  }
}