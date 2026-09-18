import { CommonModule } from '@angular/common';
import { AfterViewInit, Component } from '@angular/core';
import { RouterModule } from '@angular/router';
import { SharedModule } from '../../../shared/shared.module';
import { ApiService } from '../../../shared/services/api.service';
import { BasePageComponent } from '../../../core/base-page.component';
import { Project } from '../../../shared/models/project.model';
import { SubscriptionInfo } from '../../../shared/models/subscription.model';
import { takeUntil } from 'rxjs';
import { addressAbrev } from '../../../shared/utils';
import { TRADING_STRATEGIES } from '../../../shared/data/constants';
import { WatcherService } from '../../../shared/services/watcher.service';

@Component({
  selector: 'app-list-projects',
  templateUrl: './list-projects.component.html',
  styleUrl: './list-projects.component.scss',
  imports: [CommonModule, RouterModule, SharedModule],
})
export class ListProjectsComponent extends BasePageComponent implements AfterViewInit {
  tradingStrategies = TRADING_STRATEGIES;
  projects: Project[] = [];
  subscriptions = new Map<number, SubscriptionInfo>();
  addressAbrev = addressAbrev;
  constructor(private apiService: ApiService, private watcherService: WatcherService) {
    super()
  }

  ngAfterViewInit(): void {
    this.getProjects();
    this.watcherService.$projectStatus.subscribe(response => {
      if (response.TaskStatusUpdate) {
        let idx = this.projects.findIndex(project => project.id === response.TaskStatusUpdate.project_id);
        if (idx >= 0) {
          this.projects[idx] = {
            ...this.projects[idx],
            status: response.TaskStatusUpdate.status
          }
        }
      }
    });
  }

  getProjects() {
    this.apiService.getProjects().pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.projects = response.data;
    });
    this.apiService.getMySubscriptions().pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      for (const [projectId, sub] of Object.entries(response.data)) {
        this.subscriptions.set(+projectId, sub);
      }
    });
  }

  getExplorerUrl(network: string): string {
    return network === 'base' ? 'https://basescan.org' : 'https://solscan.io';
  }

  public getTradingStrategory(id: string) {
    let found = this.tradingStrategies.find(tradingStrategy => tradingStrategy.id === id);
    return found ? found.label : 'N/A';
  }
}
