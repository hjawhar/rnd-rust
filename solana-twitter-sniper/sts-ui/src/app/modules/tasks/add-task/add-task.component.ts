import { AfterViewInit, ChangeDetectorRef, Component, inject, signal } from '@angular/core';
import { FormGroup, FormControl, Validators } from '@angular/forms';
import { MatSnackBar } from '@angular/material/snack-bar';
import { ActivatedRoute, RouterModule } from '@angular/router';
import { forkJoin, takeUntil } from 'rxjs';
import { BasePageComponent } from '../../../core/base-page.component';
import { UpdateTaskPayload } from '../../../shared/models/task.model';
import { ApiService } from '../../../shared/services/api.service';
import { WatcherService } from '../../../shared/services/watcher.service';
import { CommonModule } from '@angular/common';
import { SharedModule } from '../../../shared/shared.module';
import { Wallet } from '../../../shared/models/wallet.model';
import { Server } from '../../../shared/models/server.model';
import { BlockLeader } from '../../../shared/models/block_leader.model';
import { Pool } from '../../../shared/models/pool.model';
import { LiveAnnouncer } from '@angular/cdk/a11y';
import { MatChipInputEvent } from '@angular/material/chips';
import { isEmptyOrNull } from '../../../shared/utils';

@Component({
  selector: 'app-add-task',
  imports: [CommonModule, SharedModule, RouterModule],
  templateUrl: './add-task.component.html',
  styleUrl: './add-task.component.scss'
})
export class AddTaskComponent extends BasePageComponent implements AfterViewInit {
  taskConfiguration = new FormGroup({
    twitter_strategy: new FormControl<string[]>([]),
    twitter_api: new FormControl<string | null>('EXTREME'),
    twitter_handle: new FormControl<string | null>(''),
    wallet_id: new FormControl<number | null>(null),
    servers: new FormControl<string[]>([]),
    block_leaders: new FormControl<string[]>([]),
    value: new FormControl<number | null>(0.01),
    tip: new FormControl<number | null>(0.002),
    slippage: new FormControl<number>(50, [Validators.required, Validators.min(1)]),
    tries: new FormControl<number>(1, [Validators.required, Validators.min(1)]),
    frontrunning_protection: new FormControl(true, [Validators.required]),
    enable_alerts: new FormControl(true, [Validators.required]),
    selected_pool: new FormControl('EXCLUDE_PUMPFUN', [Validators.required]),
    twitter_handle_checker: new FormControl<string | null>(''),
    twitter_token_override: new FormControl<string | null>(''),
    words: new FormControl<string[]>([])
  });

  id: number | undefined;
  wallets: Wallet[] = [];
  servers: Server[] = [];
  pools: Pool[] = [];
  block_leaders: BlockLeader[] = [];
  twitter_strategies = ['TWEET', 'RETWEET', 'QUOTE', 'REPLY'];
  twitter_apis = ['EXTREME', 'NORMAL']

  constructor(private apiService: ApiService, private route: ActivatedRoute, private snackbar: MatSnackBar, private watcherService: WatcherService, private cdr: ChangeDetectorRef) {
    super();
  }

  ngAfterViewInit(): void {
    forkJoin([
      this.apiService.getServers(),
      this.apiService.getBlockLeaders(),
      this.apiService.getWallets(),
      this.apiService.getPools()
    ]).subscribe(responses => {
      this.servers = responses[0].data;
      this.block_leaders = responses[1].data;
      this.wallets = responses[2].data;
      this.pools = responses[3].data;
      this.route.params.pipe(takeUntil(this.componentDestroyed$)).subscribe(params => {
        if (params && params['id'] && !isNaN(+params['id'])) {
          this.id = +params['id'];
          this.cdr.detectChanges();
          this.apiService.getTask(this.id).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
            if (response.data) {
              let { twitter_handle, twitter_api, twitter_strategy, twitter_handle_checker, twitter_token_override, words, wallet_id, value, tip, slippage, tries, frontrunning_protection, enable_alerts, servers, block_leaders, selected_pool } = response.data;
              this.taskConfiguration.get('twitter_handle')?.setValue(twitter_handle);
              this.taskConfiguration.get('twitter_api')?.setValue(twitter_api);
              this.taskConfiguration.get('twitter_strategy')?.setValue(twitter_strategy ? twitter_strategy.split(',') : []);
              this.taskConfiguration.get('twitter_handle_checker')?.setValue(twitter_handle_checker);
              this.taskConfiguration.get('twitter_token_override')?.setValue(twitter_token_override);
              this.taskConfiguration.get('words')?.setValue(words ? words!.split(',') : []);
              this.taskConfiguration.get('wallet_id')?.setValue(wallet_id ? +wallet_id : null);
              this.taskConfiguration.get('value')?.setValue(value ? +value : null);
              this.taskConfiguration.get('tip')?.setValue(tip ? +tip : null);
              this.taskConfiguration.get('slippage')?.setValue(slippage);
              this.taskConfiguration.get('tries')?.setValue(tries);
              this.taskConfiguration.get('frontrunning_protection')?.setValue(frontrunning_protection);
              this.taskConfiguration.get('enable_alerts')?.setValue(enable_alerts);
              this.taskConfiguration.get('selected_pool')?.setValue(selected_pool);
              if (servers && servers.length > 0) {
                this.taskConfiguration.get('servers')?.setValue(servers.split(','));
              }
              if (block_leaders && block_leaders.length > 0) {
                this.taskConfiguration.get('block_leaders')?.setValue(block_leaders.split(','));
              }
              this.cdr.detectChanges();
            }
          })
        }
      });
    });
  }

  public save() {
    let { twitter_handle, twitter_api, twitter_strategy, twitter_handle_checker, twitter_token_override, words, wallet_id, value, tip, slippage, tries, frontrunning_protection, enable_alerts, servers, block_leaders, selected_pool } = this.taskConfiguration.getRawValue();
    console.log(words)
    let payload: UpdateTaskPayload = {
      twitter_handle: twitter_handle ?? null,
      wallet_id: wallet_id ? +wallet_id : null,
      value: value ? +value : null,
      tip: tip ? +tip : null,
      slippage: slippage ? +slippage : 50,
      tries: tries ? +tries : 1,
      frontrunning_protection: frontrunning_protection ? frontrunning_protection : false,
      enable_alerts: enable_alerts ? enable_alerts : false,
      servers: servers && servers.length > 0 ? servers.join(',') : null,
      block_leaders: block_leaders && block_leaders.length > 0 ? block_leaders.join(',') : null,
      selected_pool: selected_pool!,
      twitter_api: twitter_api!,
      twitter_handle_checker: twitter_handle_checker ? twitter_handle_checker! : null,
      twitter_token_override: twitter_token_override ? twitter_token_override! : null,
      twitter_strategy: twitter_strategy && twitter_strategy.length > 0 ? twitter_strategy.join(',') : null,
      words: words && words.length > 0 ? words.join(',') : null
    }
    this.apiService.updateTask(this.id!, payload).pipe(takeUntil(this.componentDestroyed$)).subscribe(response => {
      this.snackbar.open(response.message, 'Dismiss', { duration: 10000 });
      this.watcherService.$updateTask.next({ id: this.id!, data: response.data });
    });
  }

  announcer = inject(LiveAnnouncer);

  removeTemplateKeyword(keyword: string) {
    let keywords = isEmptyOrNull(this.taskConfiguration.get('words')?.value) ? [] : this.taskConfiguration.get('words')?.value!
    const index = keywords.indexOf(keyword);
    if (index < 0) {
      return;
    }

    keywords.splice(index, 1);
    // this.announcer.announce(`removed ${keyword} from template form`);
    this.taskConfiguration.get('words')?.setValue(keywords)
  }

  addTemplateKeyword(event: MatChipInputEvent): void {
    const value = (event.value || '').trim();
    let keywords = isEmptyOrNull(this.taskConfiguration.get('words')?.value) ? [] : this.taskConfiguration.get('words')?.value!

    // Add our keyword
    if (value) {
      keywords.push(value);
      this.taskConfiguration.get('words')?.setValue(keywords);
      // this.announcer.announce(`added ${value} to template form`);
    }

    // Clear the input value
    event.chipInput!.clear();
  }

  public templateKeywords() {
    return isEmptyOrNull(this.taskConfiguration.get('words')?.value) ? [] : this.taskConfiguration.get('words')?.value!
  }

  public displayTwitterHandleChecker() {
    let strategies = isEmptyOrNull(this.taskConfiguration.get('twitter_strategy')?.value) ? [] : this.taskConfiguration.get('twitter_strategy')?.value!;
    return strategies.indexOf('RETWEET') >= 0 || strategies.indexOf('QUOTE') >= 0 || strategies.indexOf('REPLY') >= 0
  }
}
