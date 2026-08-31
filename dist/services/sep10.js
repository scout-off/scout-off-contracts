"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.issueSep10Token = issueSep10Token;
const uuid_1 = require("uuid");
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
function issueSep10Token(payload, secret) {
    return jsonwebtoken_1.default.sign(payload, secret, {
        expiresIn: "7d",
        jwtid: (0, uuid_1.v4)(),
    });
}
