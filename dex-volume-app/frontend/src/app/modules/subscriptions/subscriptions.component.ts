import { CommonModule } from '@angular/common';
import { AfterViewInit, Component } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { MatSnackBar } from '@angular/material/snack-bar';
import { RouterModule } from '@angular/router';
import { takeUntil } from 'rxjs';
import { BasePageComponent } from '../../core/base-page.component';
import { ApiService } from '../../shared/services/api.service';
import { SharedModule } from '../../shared/shared.module';
import {
  CreatePaymentPayload,
  CreateSubscriptionPayload,
  Payment,
  SubscriptionWithProject,
  UpdateSubscriptionPayload,
} from '../../shared/models/subscription.model';
import { Project } from '../../shared/models/project.model';
import { addressAbrev } from '../../shared/utils';

type SubFilter = 'all' | 'overdue' | 'active' | 'suspended' | 'cancelled';

@Component({
  selector: 'app-subscriptions',
  templateUrl: './subscriptions.component.html',
  styleUrls: ['./subscriptions.component.scss'],
  imports: [CommonModule, RouterModule, SharedModule, FormsModule],
})
export class SubscriptionsComponent extends BasePageComponent implements AfterViewInit {
  subscriptions: SubscriptionWithProject[] = [];
  filteredSubscriptions: SubscriptionWithProject[] = [];
  allProjects: Project[] = [];
  filter: SubFilter = 'all';
  addressAbrev = addressAbrev;

  // Expanded card state
  expandedSubId: number | null = null;
  payments: Payment[] = [];
  totalPaid: string = '0';
  paymentsLoading = false;

  // New subscription form
  showNewSubForm = false;
  newSubProjectId: number | null = null;
  newSubRate: number | null = null;
  newSubCurrency = 'USD';
  newSubDueDate = '';
  newSubNotes = '';

  // New payment form
  showPaymentForm: number | null = null;
  newPaymentAmount: number | null = null;
  newPaymentCurrency = 'USD';
  newPaymentDate = '';
  newPaymentNotes = '';

  // Edit subscription
  editSubId: number | null = null;
  editRate: number | null = null;
  editCurrency = '';
  editDueDate = '';
  editStatus = '';
  editNotes = '';

  constructor(private apiService: ApiService, private snackbar: MatSnackBar) {
    super();
  }

  ngAfterViewInit(): void {
    this.fetchSubscriptions();
    this.apiService.getAllProjects().pipe(takeUntil(this.componentDestroyed$)).subscribe(res => {
      this.allProjects = res.data;
    });
  }

  fetchSubscriptions() {
    this.apiService.getAllSubscriptions().pipe(takeUntil(this.componentDestroyed$)).subscribe(res => {
      this.subscriptions = res.data;
      this.applyFilter();
    });
  }

  applyFilter() {
    if (this.filter === 'all') {
      this.filteredSubscriptions = this.subscriptions;
    } else if (this.filter === 'overdue') {
      this.filteredSubscriptions = this.subscriptions.filter(s => s.is_overdue);
    } else {
      this.filteredSubscriptions = this.subscriptions.filter(s => s.subscription.status === this.filter);
    }
  }

  setFilter(f: SubFilter) {
    this.filter = f;
    this.applyFilter();
  }

  get projectsWithoutSub(): Project[] {
    const subProjectIds = new Set(this.subscriptions.map(s => s.subscription.project_id));
    return this.allProjects.filter(p => !subProjectIds.has(p.id));
  }

  toggleExpand(subId: number) {
    if (this.expandedSubId === subId) {
      this.expandedSubId = null;
      this.payments = [];
      return;
    }
    this.expandedSubId = subId;
    this.loadPayments(subId);
  }

  loadPayments(subId: number) {
    this.paymentsLoading = true;
    this.apiService.getPayments(subId).pipe(takeUntil(this.componentDestroyed$)).subscribe(res => {
      this.payments = res.data.payments;
      this.totalPaid = res.data.total_paid;
      this.paymentsLoading = false;
    });
  }

  // ── Create Subscription ─────────────────────────────────────

  createSubscription() {
    if (!this.newSubProjectId || !this.newSubRate || !this.newSubDueDate) return;
    const payload: CreateSubscriptionPayload = {
      monthly_rate: this.newSubRate,
      currency: this.newSubCurrency,
      next_payment_due: this.newSubDueDate,
      notes: this.newSubNotes || undefined,
    };
    this.apiService.createSubscription(this.newSubProjectId, payload)
      .pipe(takeUntil(this.componentDestroyed$))
      .subscribe({
        next: () => {
          this.snackbar.open('Subscription created', 'Dismiss', { duration: 5000 });
          this.showNewSubForm = false;
          this.newSubProjectId = null;
          this.newSubRate = null;
          this.newSubDueDate = '';
          this.newSubNotes = '';
          this.fetchSubscriptions();
        },
        error: (err) => {
          this.snackbar.open(err.error?.error || 'Failed to create subscription', 'Dismiss', { duration: 5000 });
        }
      });
  }

  // ── Edit Subscription ───────────────────────────────────────

  startEdit(sub: SubscriptionWithProject) {
    this.editSubId = sub.subscription.id;
    this.editRate = +sub.subscription.monthly_rate;
    this.editCurrency = sub.subscription.currency;
    this.editStatus = sub.subscription.status;
    this.editNotes = sub.subscription.notes || '';
    // Convert secs_since_epoch to YYYY-MM-DD
    const d = new Date(sub.subscription.next_payment_due.secs_since_epoch * 1000);
    this.editDueDate = d.toISOString().split('T')[0];
  }

  cancelEdit() {
    this.editSubId = null;
  }

  saveEdit(projectId: number) {
    if (this.editSubId === null) return;
    const payload: UpdateSubscriptionPayload = {
      monthly_rate: this.editRate ?? undefined,
      currency: this.editCurrency || undefined,
      next_payment_due: this.editDueDate || undefined,
      status: this.editStatus || undefined,
      notes: this.editNotes,
    };
    this.apiService.updateSubscription(projectId, payload)
      .pipe(takeUntil(this.componentDestroyed$))
      .subscribe({
        next: () => {
          this.snackbar.open('Subscription updated', 'Dismiss', { duration: 5000 });
          this.editSubId = null;
          this.fetchSubscriptions();
        },
        error: (err) => {
          this.snackbar.open(err.error?.error || 'Failed to update', 'Dismiss', { duration: 5000 });
        }
      });
  }

  deleteSubscription(projectId: number) {
    if (!confirm('Delete this subscription? All payment history will be lost.')) return;
    this.apiService.deleteSubscription(projectId)
      .pipe(takeUntil(this.componentDestroyed$))
      .subscribe({
        next: () => {
          this.snackbar.open('Subscription deleted', 'Dismiss', { duration: 5000 });
          this.fetchSubscriptions();
        },
        error: (err) => {
          this.snackbar.open(err.error?.error || 'Failed to delete', 'Dismiss', { duration: 5000 });
        }
      });
  }

  // ── Record Payment ──────────────────────────────────────────

  togglePaymentForm(subId: number) {
    this.showPaymentForm = this.showPaymentForm === subId ? null : subId;
    this.newPaymentAmount = null;
    this.newPaymentCurrency = 'USD';
    this.newPaymentDate = '';
    this.newPaymentNotes = '';
  }

  recordPayment(subId: number) {
    if (!this.newPaymentAmount) return;
    const payload: CreatePaymentPayload = {
      amount: this.newPaymentAmount,
      currency: this.newPaymentCurrency,
      paid_at: this.newPaymentDate || undefined,
      notes: this.newPaymentNotes || undefined,
    };
    this.apiService.createPayment(subId, payload)
      .pipe(takeUntil(this.componentDestroyed$))
      .subscribe({
        next: () => {
          this.snackbar.open('Payment recorded', 'Dismiss', { duration: 5000 });
          this.showPaymentForm = null;
          this.loadPayments(subId);
          this.fetchSubscriptions();
        },
        error: (err) => {
          this.snackbar.open(err.error?.error || 'Failed to record payment', 'Dismiss', { duration: 5000 });
        }
      });
  }

  voidPayment(subId: number, paymentId: number) {
    if (!confirm('Void this payment?')) return;
    this.apiService.deletePayment(subId, paymentId)
      .pipe(takeUntil(this.componentDestroyed$))
      .subscribe({
        next: () => {
          this.snackbar.open('Payment voided', 'Dismiss', { duration: 5000 });
          this.loadPayments(subId);
          this.fetchSubscriptions();
        },
        error: (err) => {
          this.snackbar.open(err.error?.error || 'Failed to void payment', 'Dismiss', { duration: 5000 });
        }
      });
  }

  get overdueCount(): number {
    return this.subscriptions.filter(s => s.is_overdue).length;
  }
}
