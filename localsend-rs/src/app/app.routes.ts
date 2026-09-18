import { Routes } from "@angular/router";
import { ReceiveComponent } from "./modules/receive/receive.component";
import { SendComponent } from "./modules/send/send.component";
import { SettingsComponent } from "./modules/settings/settings.component";

export const routes: Routes = [
    {
        path: 'receive',
        component: ReceiveComponent
    },
    {
        path: 'send',
        component: SendComponent
    },
    {
        path: 'settings',
        component: SettingsComponent
    },
    {
        path: '',
        pathMatch: 'full',
        redirectTo: 'receive'
    }
];
