"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const supertest_1 = __importDefault(require("supertest"));
const app_1 = __importDefault(require("../../src/app"));
const sep10_1 = require("../../src/services/sep10");
const db_1 = require("../../src/db");
beforeEach(async () => {
    await db_1.prisma.revoked_tokens.deleteMany();
});
afterAll(async () => {
    await db_1.prisma.$disconnect();
});
describe("Token revocation through real SEP-10 flow", () => {
    it("should include jti claim in JWT issued through sep10 signing path", async () => {
        const token = (0, sep10_1.issueSep10Token)({ sub: "user123", role: "validator" }, "test-secret");
        const decoded = jwt.verify(token, "test-secret");
        expect(decoded.jti).toBeDefined();
        expect(typeof decoded.jti).toBe("string");
    });
    it("should allow revoking a token via the admin endpoint when jti is present", async () => {
        const token = (0, sep10_1.issueSep10Token)({ sub: "user123", role: "validator" }, "test-secret");
        await (0, supertest_1.default)(app_1.default)
            .post("/api/admin/tokens/revoke")
            .set("Authorization", `Bearer ${token}`)
            .expect(200)
            .expect((res) => {
            expect(res.body.revoked).toBe(true);
        });
    });
    it("should block the revoked token on subsequent requests", async () => {
        const token = (0, sep10_1.issueSep10Token)({ sub: "user123", role: "validator" }, "test-secret");
        await (0, supertest_1.default)(app_1.default)
            .post("/api/admin/tokens/revoke")
            .set("Authorization", `Bearer ${token}`);
        const response = await (0, supertest_1.default)(app_1.default)
            .get("/api/tokens/me")
            .set("Authorization", `Bearer ${token}`)
            .expect(401);
        expect(response.body.error).toContain("revoked");
    });
    it("should return 400 if token does not contain jti claim (manual test helper tokens)", async () => {
        const manualToken = jwt.sign({ sub: "user123", role: "validator" }, "test-secret");
        await (0, supertest_1.default)(app_1.default)
            .post("/api/admin/tokens/revoke")
            .send({ token: manualToken })
            .expect(400)
            .expect((res) => {
            expect(res.body.error).toContain("jti claim");
        });
    });
});
