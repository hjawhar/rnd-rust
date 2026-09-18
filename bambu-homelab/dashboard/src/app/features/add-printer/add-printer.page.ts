import { Component, inject, signal } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { PrinterService } from '../../core/services/printer.service';

@Component({
  selector: 'app-add-printer',
  imports: [RouterLink, FormsModule],
  template: `
    <div class="p-6 max-w-lg mx-auto">
      <div class="mb-4">
        <a routerLink="/" class="text-sm text-primary hover:text-primary-light">← Back</a>
      </div>

      <h1 class="text-2xl font-bold text-text mb-6">Add Printer</h1>

      <form (ngSubmit)="submit()" class="space-y-4">
        <div>
          <label class="block text-sm text-text-muted mb-1">IP Address</label>
          <input type="text" [(ngModel)]="form.ip" name="ip" required
                 placeholder="192.168.1.100"
                 class="w-full px-3 py-2 bg-surface-lighter border border-border rounded-lg text-text placeholder-text-muted/50 focus:border-primary focus:outline-none" />
        </div>

        <div>
          <label class="block text-sm text-text-muted mb-1">Serial Number</label>
          <input type="text" [(ngModel)]="form.serial" name="serial" required
                 placeholder="e.g. 00M09A..."
                 class="w-full px-3 py-2 bg-surface-lighter border border-border rounded-lg text-text placeholder-text-muted/50 focus:border-primary focus:outline-none" />
        </div>

        <div>
          <label class="block text-sm text-text-muted mb-1">LAN Access Code</label>
          <input type="text" [(ngModel)]="form.access_code" name="access_code" required
                 placeholder="From printer touchscreen"
                 class="w-full px-3 py-2 bg-surface-lighter border border-border rounded-lg text-text placeholder-text-muted/50 focus:border-primary focus:outline-none" />
        </div>

        <div>
          <label class="block text-sm text-text-muted mb-1">Printer Name</label>
          <input type="text" [(ngModel)]="form.name" name="name" required
                 placeholder="My X1C"
                 class="w-full px-3 py-2 bg-surface-lighter border border-border rounded-lg text-text placeholder-text-muted/50 focus:border-primary focus:outline-none" />
        </div>

        <div>
          <label class="block text-sm text-text-muted mb-1">Model</label>
          <select [(ngModel)]="form.model" name="model"
                  class="w-full px-3 py-2 bg-surface-lighter border border-border rounded-lg text-text focus:border-primary focus:outline-none">
            <option value="X1C">X1 Carbon</option>
            <option value="X1">X1</option>
            <option value="P1S">P1S</option>
            <option value="P1P">P1P</option>
            <option value="A1">A1</option>
            <option value="A1 Mini">A1 Mini</option>
          </select>
        </div>

        @if (error()) {
          <div class="p-3 bg-error/10 border border-error/30 rounded-lg text-sm text-error">
            {{ error() }}
          </div>
        }

        @if (success()) {
          <div class="p-3 bg-success/10 border border-success/30 rounded-lg text-sm text-success">
            Printer registered successfully!
          </div>
        }

        <div class="flex gap-3 pt-2">
          <button type="submit" [disabled]="saving()"
                  class="flex-1 px-4 py-2 bg-primary text-white rounded-lg hover:bg-primary-dark transition-colors text-sm font-medium disabled:opacity-50">
            {{ saving() ? 'Saving...' : 'Add Printer' }}
          </button>
          <a routerLink="/"
             class="px-4 py-2 bg-surface-lighter text-text-muted rounded-lg hover:text-text transition-colors text-sm font-medium text-center">
            Cancel
          </a>
        </div>
      </form>
    </div>
  `,
})
export class AddPrinterPage {
  private printerService = inject(PrinterService);
  private router = inject(Router);

  form = {
    ip: '',
    serial: '',
    access_code: '',
    name: '',
    model: 'X1C',
  };

  saving = signal(false);
  error = signal<string | null>(null);
  success = signal(false);

  submit() {
    this.error.set(null);
    this.success.set(false);

    if (!this.form.ip || !this.form.serial || !this.form.access_code || !this.form.name) {
      this.error.set('All fields are required');
      return;
    }

    const ipPattern = /^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$/;
    if (!ipPattern.test(this.form.ip)) {
      this.error.set('Invalid IP address format');
      return;
    }

    this.saving.set(true);

    this.printerService.addPrinter(this.form).subscribe({
      next: () => {
        this.success.set(true);
        setTimeout(() => this.router.navigate(['/']), 1000);
      },
      error: (err) => {
        this.saving.set(false);
        this.error.set(err.message || 'Failed to add printer');
      },
    });
  }
}
