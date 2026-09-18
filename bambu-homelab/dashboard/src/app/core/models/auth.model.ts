export interface LoginRequest {
  username: string;
  password: string;
}

export interface LoginResponse {
  token: string;
  username: string;
  role: string;
}

export interface ChangePasswordRequest {
  current_password: string;
  new_password: string;
}
