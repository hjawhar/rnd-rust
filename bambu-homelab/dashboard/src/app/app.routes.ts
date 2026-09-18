import { Routes } from '@angular/router';
import { authGuard, adminGuard } from './core/auth.guard';

export const routes: Routes = [
  {
    path: 'login',
    loadComponent: () =>
      import('./features/login/login.page').then(m => m.LoginPage),
  },
  {
    path: '',
    canActivate: [authGuard],
    loadComponent: () =>
      import('./features/dashboard/dashboard.page').then(m => m.DashboardPage),
  },
  {
    path: 'printers/add',
    canActivate: [adminGuard],
    loadComponent: () =>
      import('./features/add-printer/add-printer.page').then(m => m.AddPrinterPage),
  },
  {
    path: 'printers/:id',
    canActivate: [authGuard],
    loadComponent: () =>
      import('./features/printer-detail/printer-detail.page').then(m => m.PrinterDetailPage),
  },
  {
    path: 'fleet',
    canActivate: [adminGuard],
    loadComponent: () =>
      import('./features/fleet/fleet.page').then(m => m.FleetPage),
  },
  {
    path: 'admin/users',
    canActivate: [adminGuard],
    loadComponent: () =>
      import('./features/admin/users.page').then(m => m.UsersPage),
  },
];
