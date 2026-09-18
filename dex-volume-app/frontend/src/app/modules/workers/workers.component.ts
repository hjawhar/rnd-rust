import { Component, AfterViewInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { RouterModule } from '@angular/router';
import { SharedModule } from '../../shared/shared.module';
import { BasePageComponent } from '../../core/base-page.component';
import { ApiService } from '../../shared/services/api.service';
import { takeUntil } from 'rxjs';

@Component({
    selector: 'app-workers',
    standalone: true,
    imports: [CommonModule, SharedModule, RouterModule],
    templateUrl: './workers.component.html',
    styleUrl: './workers.component.scss'
})
export class WorkersComponent extends BasePageComponent implements AfterViewInit {
    workers: any = { sol: [], evm: [] };
    loading = true;

    private apiService = inject(ApiService);

    ngAfterViewInit() {
        this.loadWorkers();
    }

    loadWorkers() {
        this.loading = true;
        this.apiService.getWorkers()
            .pipe(takeUntil(this.componentDestroyed$))
            .subscribe({
                next: (data) => { this.workers = data; this.loading = false; },
                error: () => { this.loading = false; }
            });
    }

    getTotalProjects(chain: any[]): number {
        return chain.reduce((sum: number, w: any) => sum + w.projects.length, 0);
    }

    formatTimestamp(epoch: number): Date {
        return new Date(epoch * 1000);
    }
}
