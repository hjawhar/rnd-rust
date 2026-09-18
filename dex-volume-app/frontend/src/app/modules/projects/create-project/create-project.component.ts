import { CommonModule } from '@angular/common';
import { AfterViewInit, Component } from '@angular/core';
import { Router, RouterModule } from '@angular/router';
import { SharedModule } from '../../../shared/shared.module';
import { BasePageComponent } from '../../../core/base-page.component';
import { AddProjectPayload, Project } from '../../../shared/models/project.model';
import { ApiService } from '../../../shared/services/api.service';
import { FormControl, FormGroup, Validators } from '@angular/forms';
import { takeUntil } from 'rxjs';
import { MatSnackBar } from '@angular/material/snack-bar';
import { TRADING_STRATEGIES } from '../../../shared/data/constants';
import { WatcherService } from '../../../shared/services/watcher.service';
import { CommonPool } from '../../../shared/models/token.model';

@Component({
  selector: 'app-create-project',
  imports: [CommonModule, RouterModule, SharedModule],
  templateUrl: './create-project.component.html',
  styleUrl: './create-project.component.scss'
})
export class CreateProjectComponent extends BasePageComponent implements AfterViewInit {
  latestChanges: { [key: string]: string | null | undefined } = {};
  projects: Project[] = [];
  tradingStrategies = TRADING_STRATEGIES;
  networks = [
    { id: 'solana', label: 'Solana' },
    { id: 'base', label: 'Base' }
  ];
  createProjectForm = new FormGroup({
    network: new FormControl('solana', [Validators.required]),
    address: new FormControl('', [Validators.required]),
    trading_strategy: new FormControl('VOLUME_MAKER', [Validators.required]),
    pool: new FormControl('', [Validators.required])
  });

  pools: CommonPool[] = [];
  isLoading = false;

  requestId?: string;
  constructor(private apiService: ApiService, private watcherService: WatcherService, private snackbar: MatSnackBar, private router: Router) {
    super()
  }

  ngAfterViewInit(): void {
    this.createProjectForm.valueChanges.pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      if (response.address != this.latestChanges['address']) {
        this.requestId = undefined;
        this.latestChanges['address'] = response.address;

        if (this.latestChanges['address'] && this.latestChanges['address'].length >= 42 && this.latestChanges['address'].length <= 44) {
          this.createProjectForm.disable();
          this.isLoading = true;
          this.apiService.getPools(this.latestChanges['address'], response.network!).subscribe(response => {
            this.createProjectForm.enable();
            this.isLoading = false;
            const pools = response.data ?? [];
            const seen = new Set<string>();
            this.pools = pools.filter(p => seen.has(p.pool_address) ? false : (seen.add(p.pool_address), true));
          }, err => {
            this.createProjectForm.enable();
            this.isLoading = false;
          });
        } else {
          this.pools = [];
        }
      }
    });
  }

  public save() {
    const { address, pool, trading_strategy, network } = this.createProjectForm.getRawValue();
    const payload: AddProjectPayload = {
      address: address!,
      pool: pool!,
      trading_strategy: trading_strategy!,
      network: network!
    }
    this.apiService.addProject(payload).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      if (response.data) {
        this.isLoading = true;
        this.router.navigate([`/projects/${response.data.id}`]);
        this.snackbar.open(`Successfully created new project`, 'Dismiss', {
          duration: 5000,
        });
      } else {
        this.isLoading = false;

      }
    });
  }

  getTokenSymbol = (token: string) => {
    switch (token) {
      case 'So11111111111111111111111111111111111111112':
        return 'WSOL';
      case 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v':
        return 'USDC';
      default:
        return token;
    }
  }

  formatLiquidity(value: number): string {
    if (value >= 1_000_000) return (value / 1_000_000).toFixed(2) + 'M';
    if (value >= 1_000) return (value / 1_000).toFixed(2) + 'K';
    return value.toFixed(2);
  }
}