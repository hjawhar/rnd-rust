import { Routes } from '@angular/router';
import { LoginComponent } from './modules/login/login.component';
import { AuthGuard } from './shared/auth/auth.guard';
import { MainComponent } from './modules/main/main.component';
import { DashboardOverviewComponent } from './modules/dashboard-overview/dashboard-overview.component';
import { ManualBuyComponent } from './modules/manual-buy/manual-buy.component';
import { ListWalletsComponent } from './modules/wallets/list-wallets/list-wallets.component';
import { ListTasksComponent } from './modules/tasks/list-tasks/list-tasks.component';
import { AddTaskComponent } from './modules/tasks/add-task/add-task.component';
import { ListUsersComponent } from './modules/users/list-users/list-users.component';

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
                path: '',
                component: DashboardOverviewComponent,
                children: [
                    {
                        path: 'manual-buy',
                        component: ManualBuyComponent
                    },
                    {
                        path: 'tasks',
                        component: ListTasksComponent,
                    },
                    {
                        path: 'tasks/:id',
                        component: AddTaskComponent,
                    },
                    {
                        path: 'wallets',
                        component: ListWalletsComponent
                    },
                    {
                        path: 'users',
                        component: ListUsersComponent
                    },
                    {
                        path: '',
                        pathMatch: 'full',
                        redirectTo: 'tasks'
                    },
                ]
            },
        ]
    },
];
