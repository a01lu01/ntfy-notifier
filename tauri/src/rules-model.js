export const MATCH_MODE_LABELS = {
  keyword_only: "关键词后",
  whole_text: "全文",
  both: "关键词后+全文回退"
};

export function newRule() {
  return {
    id: crypto.randomUUID(),
    name: "",
    keywords: [],
    min_length: 4,
    max_length: 8,
    match_mode: "both",
    enabled: true
  };
}

export function parseKeywords(input) {
  const seen = new Set();
  const out = [];
  for (const part of String(input || "").split(/[，,、\s]+/)) {
    const kw = part.trim();
    if (kw && !seen.has(kw)) {
      seen.add(kw);
      out.push(kw);
    }
  }
  return out;
}

export function validateRule(rule) {
  if (!String(rule.name || "").trim()) return "请填写规则名称";
  if (!Array.isArray(rule.keywords) || rule.keywords.length === 0) {
    return "请至少填写一个触发关键词";
  }
  const min = Number(rule.min_length);
  const max = Number(rule.max_length);
  if (!Number.isInteger(min) || !Number.isInteger(max) || min < 1 || max < 1) {
    return "数字位数必须是正整数";
  }
  if (min > max) return "最小位数不能大于最大位数";
  return null;
}

export function ruleSummary(rule) {
  const keywords = (rule.keywords || []).join(", ");
  const length = `${rule.min_length}-${rule.max_length}位`;
  const mode = MATCH_MODE_LABELS[rule.match_mode] || rule.match_mode || "";
  return `${keywords} · ${length} · ${mode}`;
}
