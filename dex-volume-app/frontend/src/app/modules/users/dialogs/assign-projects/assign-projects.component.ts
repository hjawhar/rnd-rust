import { Component, inject, OnInit } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatDialogTitle, MatDialogContent, MatDialogRef, MAT_DIALOG_DATA } from '@angular/material/dialog';
import { MatCheckboxModule } from '@angular/material/checkbox';
import { SharedModule } from '../../../../shared/shared.module';
import { ApiService } from '../../../../shared/services/api.service';
import { Project } from '../../../../shared/models/project.model';
import { ProjectAccess } from '../../../../shared/models/user.model';
import { forkJoin } from 'rxjs';

@Component({
  selector: 'app-assign-projects',
  imports: [CommonModule, SharedModule, MatDialogTitle, MatDialogContent, MatCheckboxModule],
  templateUrl: './assign-projects.component.html',
  styleUrl: './assign-projects.component.scss'
})
export class AssignProjectsComponent implements OnInit {
  readonly dialogRef = inject(MatDialogRef<AssignProjectsComponent>);
  readonly data: { userId: number } = inject(MAT_DIALOG_DATA);
  private apiService = inject(ApiService);

  projects: Project[] = [];
  grantedProjectIds: Set<number> = new Set();
  loading = true;
  toggling: Set<number> = new Set();

  ngOnInit(): void {
    this.loadData();
  }

  private loadData(): void {
    forkJoin({
      projects: this.apiService.getAllProjects(),
    }).subscribe({
      next: ({ projects }) => {
        this.projects = projects.data;
        // Now fetch access for each project to find which ones this user has
        const accessChecks = this.projects.map(p =>
          this.apiService.getProjectAccess(p.id).toPromise()
        );
        Promise.all(accessChecks).then(results => {
          results.forEach((result, i) => {
            if (result?.data?.some((a: ProjectAccess) => a.user_id === this.data.userId)) {
              this.grantedProjectIds.add(this.projects[i].id);
            }
          });
          this.loading = false;
        });
      },
      error: () => {
        this.loading = false;
      }
    });
  }

  isOwner(project: Project): boolean {
    return project.user_id === this.data.userId;
  }

  isGranted(projectId: number): boolean {
    return this.grantedProjectIds.has(projectId);
  }

  toggleAccess(project: Project): void {
    if (this.toggling.has(project.id)) return;
    this.toggling.add(project.id);

    if (this.isGranted(project.id)) {
      this.apiService.revokeProjectAccess(this.data.userId, project.id).subscribe({
        next: () => {
          this.grantedProjectIds.delete(project.id);
          this.toggling.delete(project.id);
        },
        error: () => {
          this.toggling.delete(project.id);
        }
      });
    } else {
      this.apiService.grantProjectAccess(this.data.userId, project.id).subscribe({
        next: () => {
          this.grantedProjectIds.add(project.id);
          this.toggling.delete(project.id);
        },
        error: () => {
          this.toggling.delete(project.id);
        }
      });
    }
  }
}
