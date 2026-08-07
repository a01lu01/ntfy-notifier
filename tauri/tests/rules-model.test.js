import test from "node:test";
import assert from "node:assert/strict";

import {
  MATCH_MODE_LABELS,
  newRule,
  parseKeywords,
  ruleSummary,
  validateRule
} from "../src/rules-model.js";

test("newRule returns enabled both-mode default template", () => {
  const rule = newRule();
  assert.equal(rule.min_length, 4);
  assert.equal(rule.max_length, 8);
  assert.equal(rule.match_mode, "both");
  assert.equal(rule.enabled, true);
  assert.equal(rule.keywords.length, 0);
  assert.equal(rule.name, "");
  assert.ok(rule.id.length > 0);
});

test("parseKeywords splits on mixed separators and trims", () => {
  assert.deepEqual(parseKeywords(" 验证码，动态码, 校验码、安全码 OTP "), [
    "验证码",
    "动态码",
    "校验码",
    "安全码",
    "OTP"
  ]);
});

test("parseKeywords dedupes and drops empty parts", () => {
  assert.deepEqual(parseKeywords("验证码, 验证码,, ,"), ["验证码"]);
});

test("parseKeywords handles empty input", () => {
  assert.deepEqual(parseKeywords(""), []);
  assert.deepEqual(parseKeywords(undefined), []);
});

test("validateRule requires name", () => {
  const rule = { ...newRule(), name: "  " };
  assert.equal(validateRule(rule), "请填写规则名称");
});

test("validateRule requires at least one keyword", () => {
  const rule = { ...newRule(), name: "规则" };
  assert.equal(validateRule(rule), "请至少填写一个触发关键词");
});

test("validateRule rejects min greater than max", () => {
  const rule = {
    ...newRule(),
    name: "规则",
    keywords: ["验证码"],
    min_length: 8,
    max_length: 4
  };
  assert.equal(validateRule(rule), "最小位数不能大于最大位数");
});

test("validateRule rejects non-positive lengths", () => {
  const rule = {
    ...newRule(),
    name: "规则",
    keywords: ["验证码"],
    min_length: 0,
    max_length: 8
  };
  assert.equal(validateRule(rule), "数字位数必须是正整数");
});

test("validateRule accepts valid rule", () => {
  const rule = { ...newRule(), name: "规则", keywords: ["验证码"] };
  assert.equal(validateRule(rule), null);
});

test("ruleSummary joins keywords, length range and mode label", () => {
  const rule = {
    ...newRule(),
    keywords: ["验证码", "OTP"],
    min_length: 6,
    max_length: 6,
    match_mode: "keyword_only"
  };
  assert.equal(ruleSummary(rule), "验证码, OTP · 6-6位 · 关键词后");
});

test("MATCH_MODE_LABELS covers all supported modes", () => {
  assert.deepEqual(Object.keys(MATCH_MODE_LABELS).sort(), [
    "both",
    "keyword_only",
    "whole_text"
  ]);
});
