import type { UserIdentity } from "../middleware/auth";

declare global {
  namespace Express {
    interface Request {
      user?: UserIdentity;
    }
  }
}

export {};
