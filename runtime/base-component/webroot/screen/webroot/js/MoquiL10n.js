/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

/*
 * The browser copy of ec.l10n: format, formatCurrency, formatCurrencyNoSymbol and localize. It is the formatter of
 * crates/moqui-lib (format.rs and datefmt.rs) in JavaScript: the Java DecimalFormat pattern subset and the Java date pattern
 * subset, HALF_EVEN rounding on decimal text (0.125 at two places is 0.12, a number is never rounded as a float), and the
 * separators of a small language table. The vectors in crates/moqui-lib/testdata/format-vectors.json run against both,
 * for this file in MoquiL10n.test.js.
 *
 * In the browser this installs moqui.l10n. Node requires it and gets the same object.
 *
 *   moqui.l10n.format(value, pattern, type)       type is optional: number, timestamp, date or time
 *   moqui.l10n.formatCurrency(amount, uomId, fractionDigits)
 *   moqui.l10n.formatCurrencyNoSymbol(amount, uomId, fractionDigits)
 *   moqui.l10n.localize(original)                 from the catalog of the locale (loadCatalog), else the text itself
 */
(function (root, factory) {
    "use strict";
    const l10n = factory();
    if (typeof module === "object" && module.exports) module.exports = l10n;
    else {
        root.moqui = root.moqui || {};
        root.moqui.l10n = l10n;
    }
})(globalThis, function () {
    "use strict";

    const CURRENCY_SIGN = "\u00a4";
    const DEFAULT_MAX_FRACTION = 12;

    const state = { locale: "en_US", timeZone: "UTC", catalog: {}, currencies: {} };

    // ---- decimals ----

    const TEN = 10n;

    // The value as { negative, coefficient (BigInt), scale }, or null when it is not a number.
    const parseDecimal = (value) => {
        if (value === null || value === undefined) return null;
        let text;
        if (typeof value === "number") {
            if (!Number.isFinite(value)) return null;
            text = String(value);
        } else if (typeof value === "bigint") text = value.toString();
        else text = String(value).trim().replace(/,/g, "");
        const match = /^([+-]?)(\d*)(?:\.(\d*))?(?:[eE]([+-]?\d+))?$/.exec(text);
        if (!match) return null;
        const whole = match[2] || "";
        const fraction = match[3] || "";
        if (whole === "" && fraction === "") return null;
        let coefficient = BigInt(whole + fraction || "0");
        let scale = fraction.length - (match[4] ? Number(match[4]) : 0);
        if (scale < 0) {
            coefficient *= TEN ** BigInt(-scale);
            scale = 0;
        }
        return { negative: match[1] === "-", coefficient, scale };
    };

    // Rounded half-even to scale places.
    const roundHalfEven = (dec, places) => {
        if (dec.scale <= places) return dec;
        const divisor = TEN ** BigInt(dec.scale - places);
        let quotient = dec.coefficient / divisor;
        const remainder = dec.coefficient % divisor;
        const half = divisor / 2n;
        if (remainder > half || (remainder === half && quotient % 2n === 1n)) quotient += 1n;
        return { negative: dec.negative, coefficient: quotient, scale: places };
    };

    // ---- separators ----

    const separators = (locale) => {
        const parts = String(locale || "").split(/[_-]/);
        const language = parts[0] || "";
        const region = parts[1] || "";
        if ((language === "de" || language === "it") && region === "CH") return ["\u2019", "."];
        if (language === "fr" && region === "CH") return ["\u202f", ","];
        if (language === "es" && region === "MX") return [",", "."];
        if (["de", "es", "it", "nl", "pt", "id", "tr", "da", "el", "ro", "hr", "sl"].includes(language)) return [".", ","];
        if (language === "fr") return ["\u202f", ","];
        if (["ru", "pl", "cs", "sk", "sv", "fi", "nb", "no", "uk", "bg", "hu", "lt", "lv"].includes(language)) return ["\u00a0", ","];
        return [",", "."];
    };

    const SUFFIX_LANGUAGES = ["de", "fr", "es", "it", "nl", "pt", "ru", "pl", "cs", "sk", "sv", "fi", "nb", "no", "da", "uk", "bg", "hu", "lt", "lv", "ro", "hr", "sl", "el"];

    // The currency pattern of a locale, as the JDK's CLDR data has it; number is the digits part such as #,##0.00
    const currencyPattern = (locale, number) => {
        const parts = String(locale || "").split(/[_-]/);
        const language = parts[0] || "";
        const region = parts[1] || "";
        if (language === "nl" && region === "NL") return CURRENCY_SIGN + "\u00a0" + number + ";" + CURRENCY_SIGN + "\u00a0-" + number;
        if ((language === "de" || language === "it") && region === "CH") return CURRENCY_SIGN + "\u00a0" + number + ";" + CURRENCY_SIGN + "-" + number;
        if (language === "pt" && region === "BR") return CURRENCY_SIGN + "\u00a0" + number;
        if (language === "es" && region === "MX") return CURRENCY_SIGN + number;
        if (SUFFIX_LANGUAGES.includes(language)) return number + "\u00a0" + CURRENCY_SIGN;
        return CURRENCY_SIGN + number;
    };

    // ---- number patterns ----

    const splitNegative = (pattern) => {
        const halves = [""];
        let quoted = false;
        for (const ch of pattern) {
            if (ch === "'") quoted = !quoted;
            if (ch === ";" && !quoted && halves.length === 1) {
                halves.push("");
                continue;
            }
            halves[halves.length - 1] += ch;
        }
        return halves;
    };

    const parseHalf = (half, symbol, whole) => {
        const layout = { prefix: "", suffix: "", minInt: 0, minFrac: 0, maxFrac: 0, group: 0, scale: 0 };
        let section = 0;
        let inFraction = false;
        let lastGroupAt = null;
        let intDigits = 0;
        const chars = Array.from(half);
        const pushAffix = (text) => {
            if (section === 0) layout.prefix += text;
            else {
                section = 2;
                layout.suffix += text;
            }
        };
        let index = 0;
        while (index < chars.length) {
            const ch = chars[index];
            index += 1;
            if (ch === "'") {
                let literal = "";
                if (chars[index] === "'") {
                    literal = "'";
                    index += 1;
                } else {
                    while (index < chars.length) {
                        if (chars[index] === "'") {
                            if (chars[index + 1] !== "'") break;
                            index += 1;
                        }
                        literal += chars[index];
                        index += 1;
                    }
                    index += 1;
                }
                pushAffix(literal);
                continue;
            }
            if ("#0,.".includes(ch) && section < 2) {
                section = 1;
                if (ch === "#") {
                    if (inFraction) layout.maxFrac += 1;
                    else intDigits += 1;
                } else if (ch === "0") {
                    if (inFraction) {
                        layout.maxFrac += 1;
                        layout.minFrac += 1;
                    } else {
                        intDigits += 1;
                        layout.minInt += 1;
                    }
                } else if (ch === ",") lastGroupAt = intDigits;
                else inFraction = true;
                continue;
            }
            if (ch === "E" && section === 1) throw new Error("exponent number pattern is not supported: " + whole);
            let text = ch;
            if (ch === CURRENCY_SIGN) text = symbol;
            else if (ch === "%") {
                layout.scale = 2;
                text = "%";
            } else if (ch === "\u2030") {
                layout.scale = 3;
                text = "\u2030";
            }
            pushAffix(text);
        }
        if (lastGroupAt !== null) layout.group = intDigits - lastGroupAt;
        return layout;
    };

    const parsePattern = (pattern, symbol) => {
        const halves = splitNegative(pattern);
        const positive = parseHalf(halves[0], symbol, pattern);
        let negative = null;
        if (halves.length > 1) {
            negative = parseHalf(halves[1], symbol, pattern);
            Object.assign(negative, { minInt: positive.minInt, minFrac: positive.minFrac, maxFrac: positive.maxFrac, group: positive.group, scale: positive.scale });
        }
        return [positive, negative];
    };

    const digitsOf = (dec, layout) => {
        let scale = dec.scale - layout.scale;
        let coefficient = dec.coefficient;
        if (scale < 0) {
            coefficient *= TEN ** BigInt(-scale);
            scale = 0;
        }
        const rounded = roundHalfEven({ negative: false, coefficient, scale }, Math.min(layout.maxFrac, 28));
        let text = rounded.coefficient.toString();
        if (rounded.scale > 0) text = text.padStart(rounded.scale + 1, "0");
        let intText = rounded.scale > 0 ? text.slice(0, text.length - rounded.scale) : text;
        let frac = rounded.scale > 0 ? text.slice(text.length - rounded.scale) : "";
        frac = frac.replace(/0+$/, "");
        while (frac.length < layout.minFrac) frac += "0";
        intText = intText.replace(/^0+/, "");
        while (intText.length < layout.minInt) intText = "0" + intText;
        // Java prints a zero integer digit when the pattern asks for none, so #.## gives 0.5 and 0
        if (intText === "") intText = "0";
        return [intText, frac];
    };

    const groupDigits = (intText, size, separator) => {
        if (size === 0 || intText.length <= size) return intText;
        const lead = intText.length % size;
        const out = [];
        if (lead > 0) out.push(intText.slice(0, lead));
        for (let at = lead; at < intText.length; at += size) out.push(intText.slice(at, at + size));
        return out.join(separator);
    };

    const renderNumber = (dec, positive, negative, locale) => {
        const [groupSep, decimalSep] = separators(locale);
        const [intText, frac] = digitsOf(dec, positive);
        const zero = /^0*$/.test(intText) && /^0*$/.test(frac);
        const negativeValue = dec.negative && !zero;
        let prefix = positive.prefix;
        let suffix = positive.suffix;
        if (negativeValue) {
            if (negative) {
                prefix = negative.prefix;
                suffix = negative.suffix;
            } else prefix = "-" + positive.prefix;
        }
        let out = prefix + groupDigits(intText, positive.group, groupSep);
        if (frac !== "") out += decimalSep + frac;
        return out + suffix;
    };

    const formatNumber = (value, pattern, locale) => {
        const dec = parseDecimal(value);
        if (!dec) throw new Error("cannot format " + String(value) + " as a number");
        const text = pattern && pattern.length ? pattern : "#,##0." + "#".repeat(DEFAULT_MAX_FRACTION);
        const [positive, negative] = parsePattern(text, "");
        return renderNumber(dec, positive, negative, locale || state.locale);
    };

    const formatCurrencyAmount = (value, symbol, fractionDigits, locale) => {
        const dec = parseDecimal(value);
        if (!dec) throw new Error("cannot format " + String(value) + " as currency");
        const fraction = fractionDigits > 0 ? "." + "0".repeat(fractionDigits) : "";
        const number = "#,##0" + fraction;
        const pattern = symbol ? currencyPattern(locale, number) : number;
        const [positive, negative] = parsePattern(pattern, symbol);
        return renderNumber(dec, positive, negative, locale);
    };

    // ---- date patterns ----

    const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

    const pad = (value, width) => {
        const text = String(Math.abs(value));
        return (value < 0 ? "-" : "") + "0".repeat(Math.max(0, width - text.length)) + text;
    };

    // width "hours" is +HH, "minutes" +HHMM, "colon" +HH:MM; X prints Z for a zero offset
    const offsetText = (seconds, width, zForZero) => {
        if (seconds === 0 && zForZero) return "Z";
        const total = Math.floor(Math.abs(seconds) / 60);
        const hours = (seconds < 0 ? "-" : "+") + pad(Math.floor(total / 60), 2);
        if (width === "hours") return hours;
        return hours + (width === "colon" ? ":" : "") + pad(total % 60, 2);
    };

    // The short zone name as SimpleDateFormat prints it (PST); a name that is only an offset prints as GMT-03:00
    const zoneText = (f) => (/^[A-Za-z]+$/.test(f.zoneName || "") ? f.zoneName : "GMT" + offsetText(f.offset, "colon", false));

    const dayOfYear = (year, month, day) => Math.round((Date.UTC(year, month - 1, day) - Date.UTC(year, 0, 1)) / 86400000) + 1;

    // fields: { year, month, day, hour, minute, second, nano, weekday (0 Sunday), offset (seconds), zone }
    const formatFields = (f, pattern) => {
        const chars = Array.from(pattern);
        let out = "";
        let index = 0;
        while (index < chars.length) {
            const ch = chars[index];
            if (ch === "'") {
                if (chars[index + 1] === "'") {
                    out += "'";
                    index += 2;
                    continue;
                }
                let end = index + 1;
                for (;;) {
                    if (end >= chars.length) throw new Error("unclosed quote in date pattern " + pattern);
                    if (chars[end] === "'" && chars[end + 1] === "'") {
                        out += "'";
                        end += 2;
                    } else if (chars[end] === "'") break;
                    else {
                        out += chars[end];
                        end += 1;
                    }
                }
                index = end + 1;
                continue;
            }
            if (!/[A-Za-z]/.test(ch)) {
                out += ch;
                index += 1;
                continue;
            }
            let count = 1;
            while (chars[index + count] === ch) count += 1;
            index += count;
            switch (ch) {
                case "G": out += f.year > 0 ? "AD" : "BC"; break;
                case "y": case "u": out += count === 2 ? pad(((f.year % 100) + 100) % 100, 2) : pad(f.year, count); break;
                case "M": case "L":
                    if (count <= 2) out += pad(f.month, count);
                    else if (count === 3) out += MONTHS[f.month - 1].slice(0, 3);
                    else out += MONTHS[f.month - 1];
                    break;
                case "d": out += pad(f.day, count); break;
                case "D": out += pad(dayOfYear(f.year, f.month, f.day), count); break;
                case "E": out += count >= 4 ? WEEKDAYS[f.weekday] : WEEKDAYS[f.weekday].slice(0, 3); break;
                case "a": out += f.hour < 12 ? "AM" : "PM"; break;
                case "H": out += pad(f.hour, count); break;
                case "k": out += pad(f.hour === 0 ? 24 : f.hour, count); break;
                case "K": out += pad(f.hour % 12, count); break;
                case "h": out += pad(f.hour % 12 === 0 ? 12 : f.hour % 12, count); break;
                case "m": out += pad(f.minute, count); break;
                case "s": out += pad(f.second, count); break;
                case "S": {
                    const nanos = pad(f.nano, 9);
                    out += nanos.slice(0, Math.min(count, 9)) + "0".repeat(Math.max(0, count - 9));
                    break;
                }
                case "Z": out += offsetText(f.offset, "minutes", false); break;
                case "X": case "x": out += offsetText(f.offset, count === 1 ? "hours" : count === 2 ? "minutes" : "colon", ch === "X"); break;
                case "z": out += zoneText(f); break;
                default: throw new Error("date pattern letter " + ch + " is not supported in " + pattern);
            }
        }
        return out;
    };

    const zoneFormatters = {};
    const zonedFields = (instant, zone) => {
        let formatter = zoneFormatters[zone];
        if (!formatter) {
            formatter = new Intl.DateTimeFormat("en-US", {
                timeZone: zone, hourCycle: "h23", year: "numeric", month: "numeric", day: "numeric",
                hour: "numeric", minute: "numeric", second: "numeric", weekday: "long", timeZoneName: "short"
            });
            zoneFormatters[zone] = formatter;
        }
        const parts = {};
        formatter.formatToParts(instant).forEach((part) => { parts[part.type] = part.value; });
        const year = Number(parts.year);
        const month = Number(parts.month);
        const day = Number(parts.day);
        const hour = Number(parts.hour) % 24;
        const minute = Number(parts.minute);
        const second = Number(parts.second);
        const asUtc = Date.UTC(year, month - 1, day, hour, minute, second);
        const offset = Math.round((asUtc - Math.floor(instant.getTime() / 1000) * 1000) / 1000);
        return {
            year, month, day, hour, minute, second, nano: ((instant.getTime() % 1000) + 1000) % 1000 * 1000000,
            weekday: WEEKDAYS.indexOf(parts.weekday), offset, zone, zoneName: parts.timeZoneName
        };
    };

    const toInstant = (value) => {
        if (value instanceof Date) return value;
        if (typeof value === "number") return new Date(value);
        const text = String(value).trim();
        // a time stamp without a zone is in the session zone only for the display; take it as UTC like the server's JSON
        const stamp = /^\d{4}-\d\d-\d\d[T ]\d\d:\d\d/.test(text) && !/(Z|[+-]\d\d:?\d\d)$/.test(text) ? text.replace(" ", "T") + "Z" : text;
        const date = new Date(stamp);
        if (Number.isNaN(date.getTime())) throw new Error("cannot format " + text + " as a time stamp");
        return date;
    };

    const formatTimestamp = (value, pattern, zone) =>
        formatFields(zonedFields(toInstant(value), zone || state.timeZone), pattern && pattern.length ? pattern : "yyyy-MM-dd HH:mm");

    const formatDate = (value, pattern) => {
        const match = /^(\d{4})-(\d\d)-(\d\d)/.exec(String(value));
        if (!match) throw new Error("cannot format " + String(value) + " as a date");
        const year = Number(match[1]);
        const month = Number(match[2]);
        const day = Number(match[3]);
        const weekday = new Date(Date.UTC(year, month - 1, day)).getUTCDay();
        return formatFields({ year, month, day, hour: 0, minute: 0, second: 0, nano: 0, weekday, offset: 0, zone: "UTC", zoneName: "UTC" }, pattern && pattern.length ? pattern : "yyyy-MM-dd");
    };

    const formatTime = (value, pattern) => {
        const match = /^(\d\d):(\d\d)(?::(\d\d)(?:\.(\d+))?)?/.exec(String(value));
        if (!match) throw new Error("cannot format " + String(value) + " as a time");
        const nano = Number(((match[4] || "") + "000000000").slice(0, 9));
        return formatFields({
            year: 1970, month: 1, day: 1, hour: Number(match[1]), minute: Number(match[2]), second: Number(match[3] || 0), nano,
            weekday: 4, offset: 0, zone: "UTC", zoneName: "UTC"
        }, pattern && pattern.length ? pattern : "HH:mm:ss");
    };

    // ---- ec.l10n ----

    const ISO_CURRENCIES = {
        USD: ["$", 2], EUR: ["\u20ac", 2], GBP: ["\u00a3", 2], JPY: ["\u00a5", 0], CNY: ["CN\u00a5", 2], INR: ["\u20b9", 2],
        CAD: ["CA$", 2], AUD: ["A$", 2], MXN: ["MX$", 2], BRL: ["R$", 2], CHF: ["CHF", 2], KRW: ["\u20a9", 0]
    };

    const currencyInfo = (uomId) => {
        if (!uomId) return { symbol: "", fractionDigits: null };
        const known = state.currencies[uomId];
        const iso = ISO_CURRENCIES[uomId];
        return {
            symbol: (known && known.symbol) || (iso ? iso[0] : uomId),
            fractionDigits: known && known.fractionDigits !== undefined && known.fractionDigits !== null ? Number(known.fractionDigits) : iso ? iso[1] : null
        };
    };

    const formatCurrencyWith = (hideSymbol) => (amount, uomId, fractionDigits) => {
        if (amount === null || amount === undefined || amount === "") return "";
        const info = currencyInfo(uomId);
        const digits = fractionDigits !== undefined && fractionDigits !== null && fractionDigits !== "" ? Number(fractionDigits) : info.fractionDigits !== null ? info.fractionDigits : 2;
        return formatCurrencyAmount(amount, hideSymbol ? "" : info.symbol, digits, state.locale);
    };

    // A value as text. A string is itself, unless a type says it is a time stamp, date, time or number.
    const format = (value, pattern, type) => {
        if (value === null || value === undefined) return "";
        const kind = type ? String(type).toLowerCase() : "";
        if (kind === "timestamp" || kind === "date-time") return formatTimestamp(value, pattern, state.timeZone);
        if (kind === "date") return formatDate(value, pattern);
        if (kind === "time") return formatTime(value, pattern);
        if (kind === "number" || kind === "bigdecimal" || kind === "long" || kind === "integer" || kind === "double" || kind === "float") {
            return formatNumber(value, pattern, state.locale);
        }
        if (typeof value === "string") return value;
        if (typeof value === "boolean") return String(value);
        if (value instanceof Date) return formatTimestamp(value, pattern, state.timeZone);
        if (typeof value === "number" || typeof value === "bigint") return formatNumber(value, pattern, state.locale);
        const json = JSON.stringify(value);
        return json.length > 128 ? JSON.stringify(value, null, 2) : json;
    };

    const localize = (original) => {
        if (original === null || original === undefined || original === "") return "";
        const text = String(original);
        const found = state.catalog[text];
        if (found !== undefined) return found;
        const hash = text.lastIndexOf("##");
        const close = text.lastIndexOf("}");
        if (hash > 0 && (close < 0 || hash > close)) {
            const base = text.slice(0, hash);
            const baseFound = state.catalog[base];
            return baseFound !== undefined ? baseFound : base;
        }
        return text;
    };

    // The catalog of a locale is the object the l10nCatalog transition answers: { locale, messages: { original: localized } }.
    // The loader is a function that takes the locale and returns a promise of that object.
    const loadCatalog = (locale, loader) => Promise.resolve(loader(locale)).then((response) => {
        const messages = response && response.messages ? response.messages : {};
        state.catalog = Object.assign({}, messages);
        return state.catalog;
    });

    return {
        format, formatNumber, formatCurrency: formatCurrencyWith(false), formatCurrencyNoSymbol: formatCurrencyWith(true),
        formatCurrencyAmount, formatTimestamp, formatDate, formatTime, localize, loadCatalog, parseDecimal, roundHalfEven,
        setLocale: (locale) => { state.locale = String(locale || "en_US").replace("-", "_"); },
        setTimeZone: (zone) => { state.timeZone = zone || "UTC"; },
        setCurrencies: (currencies) => { state.currencies = currencies || {}; },
        setCatalog: (catalog) => { state.catalog = catalog || {}; },
        get locale() { return state.locale; },
        get timeZone() { return state.timeZone; }
    };
});
