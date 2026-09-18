import { NgModule } from '@angular/core';
import { CommonModule } from '@angular/common';

import { ListWalletsComponent } from './list-wallets/list-wallets.component';
import { RouterOutlet, RouterModule } from '@angular/router';
import { SharedModule } from '../../shared/shared.module';


@NgModule({
  declarations: [
    ListWalletsComponent,
  ],
  imports: [CommonModule, RouterOutlet, RouterModule, SharedModule],

})
export class WalletsModule { }
