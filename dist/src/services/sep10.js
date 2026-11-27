"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.issueSep10Token = issueSep10Token;
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const node_crypto_1 = require("node:crypto");
function issueSep10Token(payload, secret) {
    return jsonwebtoken_1.default.sign(payload, secret, {
        expiresIn: "15m",
        jwtid: (0, node_crypto_1.randomUUID)(),
    });
}
