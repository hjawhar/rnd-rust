import { JwtHelperService } from '@auth0/angular-jwt';

export function isEmptyOrNull<T>(value: T) {
    return value === undefined || value === null || value === '';
}

export function getJwtToken() {
    return localStorage.getItem('jwt');
}

export function isLoggedIn() {
    const helper = new JwtHelperService();
    if (!isEmptyOrNull(getJwtToken())) {
        return !helper.isTokenExpired(getJwtToken());
    }
    return false;
}

export function getUser() {
    if (!isLoggedIn()) {
        return null;
    }
    const helper = new JwtHelperService();
    const decodedToken = helper.decodeToken(getJwtToken()!);
    return decodedToken;
}

export function isCustomer(): boolean {
    if (!isLoggedIn()) {
        return false;
    }
    const helper = new JwtHelperService();
    const decodedToken = helper.decodeToken(getJwtToken()!);
    if (isEmptyOrNull(decodedToken.groupid)) {
        return false;
    }
    return decodedToken.groupid === 3;
}

export function openInNewTab(url: string) {
    window.location.href = url;
}

export function deepClone<T>(data: T): T {
    return JSON.parse(JSON.stringify(data));
}

export function isLocalStorageAvailable(key: string) {
    return isEmptyOrNull(localStorage.getItem(key)) ? false : true
}

function checkNotificationPromise() {
    try {
        Notification.requestPermission().then();
    } catch (e) {
        return false;
    }

    return true;
}

export function askNotificationPermission() {
    // Let's check if the browser supports notifications
    if (!('Notification' in window)) {
        console.log("This browser does not support notifications.");
    } else if (checkNotificationPromise()) {
        Notification.requestPermission().then((permission) => {
            // handlePermission(permission);
        });
    } else {
        Notification.requestPermission((permission) => {
            // handlePermission(permission);
        });
    }
}

export function convertToNumber(input: string) {
    return Number.parseInt(input, 16);
}

export function convertToNumberDecimals(input: string, decimals: number) {
    return Number.parseInt(input, 16) / (10 ** decimals);
}


export function getNearestNumber(input: number) {
    const base = 25;
    const div = Math.floor(input / base);
    const remaining = input % base;
    const next_base = div * base;
    const next_remaining = remaining > base / 2 ? base : 0;
    return next_base + next_remaining
}

export function convertStringToJson(inputString: string): any {
    try {
        const jsonString = inputString.replace(/\\"/g, '"');
        const jsonObject = JSON.parse(jsonString);
        return jsonObject;
    } catch (error) {
        return null;
    }
}

export function makeRandomUniqueCode(lengthOfCode: number) {
    let possible = "ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890`";
    let text = "";
    for (let i = 0; i < lengthOfCode; i++) {
        text += possible.charAt(Math.floor(Math.random() * possible.length));
    }
    return text;
} 
export function addressAbrev(input: string) {
    return `${input.slice(0, 5)}...${input.slice(input.length - 6, input.length)}`
}