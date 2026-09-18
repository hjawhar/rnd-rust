import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { PrinterService } from '../../core/services/printer.service';
import { PrinterWithStatus } from '../../core/models/printer.model';

interface UserInfo {
  id: string;
  username: string;
  role: string;
}

@Component({
  selector: 'app-users',
  imports: [FormsModule, RouterLink],
  template: `
    <div class="p-6 max-w-4xl mx-auto">
      <div class="flex items-center justify-between mb-6">
        <h1 class="text-2xl font-bold text-text">User Management</h1>
        <a routerLink="/" class="text-sm text-primary hover:text-primary-light">&larr; Dashboard</a>
      </div>

      <!-- Create user -->
      <div class="bg-surface-light rounded-lg border border-border p-4 mb-6">
        <h2 class="text-sm font-semibold text-text-muted mb-3">Create User</h2>
        <form (ngSubmit)="createUser()" class="flex gap-2 flex-wrap">
          <input type="text" [(ngModel)]="newUser.username" name="username" placeholder="Username" required
                 class="px-3 py-2 bg-surface-lighter border border-border rounded text-sm text-text placeholder-text-muted/50 focus:border-primary focus:outline-none" />
          <input type="password" [(ngModel)]="newUser.password" name="password" placeholder="Password" required
                 class="px-3 py-2 bg-surface-lighter border border-border rounded text-sm text-text placeholder-text-muted/50 focus:border-primary focus:outline-none" />
          <button type="submit" class="px-4 py-2 bg-primary text-white rounded text-sm font-medium hover:bg-primary-dark">
            Create
          </button>
        </form>
        @if (error()) {
          <div class="text-xs text-error mt-2">{{ error() }}</div>
        }
      </div>

      <!-- Users list -->
      <div class="bg-surface-light rounded-lg border border-border">
        @for (user of users(); track user.id) {
          <div class="p-4 border-b border-border/50 last:border-0">
            <div class="flex items-center justify-between mb-2">
              <div class="flex items-center gap-2">
                <span class="text-text font-medium">{{ user.username }}</span>
                <span class="px-2 py-0.5 rounded-full text-xs"
                      [class]="user.role === 'admin' ? 'bg-primary/20 text-primary' : 'bg-surface-lighter text-text-muted'">
                  {{ user.role }}
                </span>
              </div>
              @if (user.role !== 'admin') {
                <button (click)="deleteUser(user)"
                        class="text-xs text-error hover:text-error/80">
                  Delete
                </button>
              }
            </div>

            @if (user.role !== 'admin') {
              <div class="mt-2 pl-4 border-l-2 border-border/50">
                <div class="flex items-center gap-2 mb-1">
                  <span class="text-xs text-text-muted">Assigned printers:</span>
                  <button (click)="showAssignDialog(user)" class="text-xs text-primary hover:text-primary-light">+ Assign</button>
                </div>
                @if (userAssignments()[user.id]; as assignments) {
                  @if (assignments.length > 0) {
                    @for (a of assignments; track a.printer_id) {
                      <div class="flex items-center justify-between py-1">
                        <span class="text-xs text-text">{{ a.printer_name }}</span>
                        <button (click)="unassign(a.printer_id, user.id)" class="text-[10px] text-error hover:text-error/80">Remove</button>
                      </div>
                    }
                  } @else {
                    <span class="text-[10px] text-text-muted/50">None</span>
                  }
                } @else {
                  <span class="text-[10px] text-text-muted/50">Loading...</span>
                }
              </div>
            }
          </div>
        }
      </div>

      <!-- Assign dialog -->
      @if (assignTarget()) {
        <div class="fixed inset-0 bg-black/50 flex items-center justify-center z-50" (click)="assignTarget.set(null)">
          <div class="bg-surface-light rounded-lg border border-border p-4 w-80" (click)="$event.stopPropagation()">
            <h3 class="text-sm font-semibold text-text mb-3">Assign printer to {{ assignTarget()!.username }}</h3>
            <div class="space-y-1 max-h-60 overflow-y-auto">
              @for (printer of availablePrinters(); track printer.id) {
                <button (click)="assignPrinter(printer.id)"
                        class="w-full text-left px-3 py-2 rounded text-sm text-text hover:bg-surface-lighter/50 transition-colors">
                  {{ printer.name }} <span class="text-text-muted text-xs">({{ printer.model }})</span>
                </button>
              }
              @if (availablePrinters().length === 0) {
                <p class="text-xs text-text-muted p-2">No printers available to assign</p>
              }
            </div>
            <button (click)="assignTarget.set(null)" class="mt-3 text-xs text-text-muted hover:text-text">Cancel</button>
          </div>
        </div>
      }
    </div>
  `,
})
export class UsersPage implements OnInit {
  private printerService = inject(PrinterService);

  users = signal<UserInfo[]>([]);
  error = signal<string | null>(null);
  newUser = { username: '', password: '' };
  printers = signal<PrinterWithStatus[]>([]);
  userAssignments = signal<Record<string, { printer_id: string; printer_name: string }[]>>({});
  assignTarget = signal<UserInfo | null>(null);

  availablePrinters = () => {
    const target = this.assignTarget();
    if (!target) return [] as PrinterWithStatus[];
    const assigned = this.userAssignments()[target.id] ?? [];
    const assignedIds = new Set(assigned.map(a => a.printer_id));
    return this.printers().filter(p => !assignedIds.has(p.id));
  };

  ngOnInit() {
    this.loadUsers();
    this.loadPrinters();
  }

  loadUsers() {
    this.printerService.listUsers().subscribe({
      next: (users) => {
        this.users.set(users);
        for (const user of users) {
          if (user.role !== 'admin') {
            this.loadUserAssignments(user.id);
          }
        }
      },
    });
  }

  loadPrinters() {
    this.printerService.listPrinters().subscribe({
      next: (printers) => this.printers.set(printers),
    });
  }

  loadUserAssignments(userId: string) {
    const printers = this.printers();
    if (printers.length === 0) {
      setTimeout(() => this.loadUserAssignments(userId), 500);
      return;
    }
    const assignments: { printer_id: string; printer_name: string }[] = [];
    let remaining = printers.length;

    for (const printer of printers) {
      this.printerService.listAssignments(printer.id).subscribe({
        next: (list) => {
          if (list.find(a => a.user_id === userId)) {
            assignments.push({ printer_id: printer.id, printer_name: printer.name });
          }
          remaining--;
          if (remaining === 0) {
            this.userAssignments.update(m => ({ ...m, [userId]: [...assignments] }));
          }
        },
        error: () => {
          remaining--;
          if (remaining === 0) {
            this.userAssignments.update(m => ({ ...m, [userId]: [...assignments] }));
          }
        },
      });
    }
  }

  createUser() {
    this.error.set(null);
    this.printerService.createUser({ ...this.newUser, role: 'user' }).subscribe({
      next: () => {
        this.newUser = { username: '', password: '' };
        this.loadUsers();
      },
      error: (err: any) => this.error.set(err.error || 'Failed to create user'),
    });
  }

  deleteUser(user: UserInfo) {
    if (!confirm(`Delete user '${user.username}'?`)) return;
    this.printerService.deleteUser(user.id).subscribe({
      next: () => this.loadUsers(),
    });
  }

  showAssignDialog(user: UserInfo) {
    this.assignTarget.set(user);
  }

  assignPrinter(printerId: string) {
    const target = this.assignTarget();
    if (!target) return;
    this.printerService.assignPrinter(printerId, target.id).subscribe({
      next: () => {
        this.assignTarget.set(null);
        this.loadUserAssignments(target.id);
      },
      error: (err: any) => this.error.set(err.error || 'Failed to assign'),
    });
  }

  unassign(printerId: string, userId: string) {
    this.printerService.unassignPrinter(printerId, userId).subscribe({
      next: () => this.loadUserAssignments(userId),
    });
  }
}
