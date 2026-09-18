import { NgModule } from '@angular/core';
import { RouterModule, Routes } from '@angular/router';
import { ListWalletsComponent } from './list-wallets/list-wallets.component';

const routes: Routes = [
  {
    path: '',
    component: ListWalletsComponent
  }
];

@NgModule({
  imports: [RouterModule.forChild(routes)],
  exports: [RouterModule]
})
export class WalletsRoutingModule { }
