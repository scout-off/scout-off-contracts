import express, { type Request, type Response } from "express";
import jwt, { type JwtPayload } from "jsonwebtoken";
import type { EvidenceGrantChecker } from "./services/evidenceGrantChecker";
import type { EvidenceDatabase } from "./routes/evidence";
import { createEvidenceRouter } from "./routes/evidence";
import { createRequireAuth, requireRole } from "./middleware/auth";

export interface AppDependencies {
  database: EvidenceDatabase;
  grantChecker: EvidenceGrantChecker;
  masterKey: Buffer;
  jwtSecret: string;
  isTokenRevoked: (jti: string) => Promise<boolean>;
}

export function createApp(dependencies: AppDependencies) {
  const app = express();
  app.disable("x-powered-by");
  app.use(express.json({ limit: "16kb" }));

  const authenticate = createRequireAuth(
    dependencies.jwtSecret,
    dependencies.isTokenRevoked
  );

  app.get("/", (_req, res) => {
    res.type("html").send(
      "<!doctype html><html><head><title>ScoutChain Evidence API</title></head>" +
        "<body><h1>ScoutChain Evidence API</h1>" +
        "<p>Evidence key release endpoint: POST " +
        "/api/evidence/:playerId/:milestoneIndex/key</p></body></html>"
    );
  });

  app.get("/api/tokens/me", authenticate, (req: Request, res: Response) => {
    const { sub, role, jti } = req.user!;
    res.json({ sub, role, jti });
  });

  app.post(
    "/api/admin/tokens/revoke",
    authenticate,
    requireRole(["admin"]),
    async (req: Request, res: Response, next) => {
      const { token } = req.body as { token?: unknown };
      if (typeof token !== "string" || token.length === 0) {
        res.status(400).json({ error: "Token is required" });
        return;
      }

      let payload: JwtPayload;
      try {
        const verified = jwt.verify(token, dependencies.jwtSecret, {
          algorithms: ["HS256"],
        });
        if (typeof verified === "string") {
          res.status(400).json({ error: "Invalid token" });
          return;
        }
        payload = verified;
      } catch {
        res.status(400).json({ error: "Invalid token" });
        return;
      }

      if (typeof payload.jti !== "string" || payload.jti.length === 0) {
        res.status(400).json({ error: "Token does not contain a jti claim" });
        return;
      }

      try {
        await dependencies.database.query(
          "INSERT INTO revoked_tokens (jti) VALUES ($1) ON CONFLICT (jti) DO NOTHING",
          [payload.jti]
        );
        res.json({ revoked: true });
      } catch (error) {
        next(error);
      }
    }
  );

  app.use(
    "/api/evidence",
    createEvidenceRouter({
      database: dependencies.database,
      grantChecker: dependencies.grantChecker,
      masterKey: dependencies.masterKey,
      authenticate,
    })
  );

  app.use(
    (
      _error: unknown,
      _req: Request,
      res: Response,
      _next: (error?: unknown) => void
    ) => {
      res.status(500).json({ error: "Internal server error" });
    }
  );

  return app;
}
