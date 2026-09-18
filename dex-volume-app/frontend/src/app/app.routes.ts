import { Routes } from '@angular/router';
import { LoginComponent } from './modules/login/login.component';
import { AuthGuard } from './shared/auth/auth.guard';
import { MainComponent } from './modules/main/main.component';
import { DashboardOverviewComponent } from './modules/dashboard-overview/dashboard-overview.component';
import { ListProjectsComponent } from './modules/projects/list-projects/list-projects.component';
import { CreateProjectComponent } from './modules/projects/create-project/create-project.component';
import { ProjectDetailsComponent } from './modules/projects/project-details/project-details.component';
import { ListUsersComponent } from './modules/users/list-users/list-users.component';
import { AuditLogsComponent } from './modules/audit-logs/audit-logs.component';
import { SubscriptionsComponent } from './modules/subscriptions/subscriptions.component';
import { WorkersComponent } from './modules/workers/workers.component';

export const routes: Routes = [
    {
        path: 'login',
        component: LoginComponent
    },
    {
        path: '',
        component: MainComponent,
        canMatch: [AuthGuard],
        children: [
            {
                path: 'users',
                component: ListUsersComponent
            },
            {
                path: 'audit',
                component: AuditLogsComponent
            },
            {
                path: 'subscriptions',
                component: SubscriptionsComponent
            },
            {
                path: 'workers',
                component: WorkersComponent
            },
            {
                path: 'projects',
                component: ListProjectsComponent
            },
            {
                path: 'projects/new',
                component: CreateProjectComponent
            },
            {
                path: 'projects/:id',
                component: ProjectDetailsComponent
            },
            {
                path: '',
                pathMatch: 'full',
                redirectTo: 'projects'
            },
        ]
    },
];
