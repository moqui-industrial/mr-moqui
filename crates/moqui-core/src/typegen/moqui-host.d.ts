// The host surface of plans/ContextObjects.md (Server host methods) and the browser host of plans/QvtServerStatic.md.
// This part does not depend on the definitions. `moqui types` writes it beside the generated files.
// Declarations only: the QuickJS host does not read this file and does not enforce it.

// ---- host values ----

/**
 * A decimal on the server: a `rust_decimal` host value, not a JavaScript number. Operators are not overloaded, so
 * `amount + tax` and `amount * tax` are type errors here. `==` and `if (amount)` still compile; `moqui lint` flags them.
 * In the unwrapped projection, the locked-down template and the browser a decimal is a string.
 */
interface Decimal {
    add(other: Decimal | number | string): Decimal;
    subtract(other: Decimal | number | string): Decimal;
    multiply(other: Decimal | number | string): Decimal;
    /** Scale `max(left, right, 10)`. */
    divide(other: Decimal | number | string): Decimal;
    eq(other: Decimal | number | string): boolean;
    /** -1, 0 or 1. */
    compare(other: Decimal | number | string): number;
    isZero(): boolean;
    /** A JavaScript number, with float rounding. */
    toNumber(): number;
    toString(): string;
    /** Throws an error that names the method to use. */
    valueOf(): never;
}

/** A timestamp: ISO-8601 text, branded so a plain string is not taken for one. */
type Timestamp = string & { readonly __moquiTimestamp: never };

/** A value that converts to a decimal parameter: the host value, or text and numbers the validation converts. */
type DecimalInput = Decimal | number | string;
/** A value that converts to a timestamp parameter. */
type TimestampInput = Timestamp | string | number;

/** A plain map with no proven keys. */
type MoquiMap = Record<string, unknown>;

/** The context stack as a map. A name no map holds reads as null on the server. */
interface MoquiContext {
    [name: string]: unknown;
}

// ---- entities ----

/** Filled by the generated `entities.d.ts`: one entry for each entity by full name, unique entity name, and short alias. */
interface MoquiEntityMap {}

/** What the generator knows of an entity: field types as the host's get returns them, primary key names, relationships. */
interface MoquiEntityShape {
    fields: object;
    pk: string;
    rel: object;
}

type MoquiEntityOf<N extends string> = N extends keyof MoquiEntityMap ? MoquiEntityMap[N] : never;
/** The field types of entity `N`, or a plain map when the entity is not loaded. */
type MoquiFields<N extends string> = [MoquiEntityOf<N>] extends [never]
    ? MoquiMap
    : MoquiEntityOf<N> extends MoquiEntityShape
      ? MoquiEntityOf<N>["fields"]
      : MoquiMap;
type MoquiFieldName<N extends string> = keyof MoquiFields<N> & string;
type MoquiPk<N extends string> = [MoquiEntityOf<N>] extends [never]
    ? string
    : MoquiEntityOf<N> extends MoquiEntityShape
      ? MoquiEntityOf<N>["pk"] & string
      : string;
type MoquiRel<N extends string> = [MoquiEntityOf<N>] extends [never]
    ? Record<string, string>
    : MoquiEntityOf<N> extends MoquiEntityShape
      ? MoquiEntityOf<N>["rel"]
      : Record<string, string>;
type MoquiRelated<N extends string, R extends string> = R extends keyof MoquiRel<N>
    ? MoquiRel<N>[R] & string
    : string;

/** What a field set accepts: the entity layer converts text to the field type on write. */
type MoquiFieldInput<T> = T extends Decimal
    ? DecimalInput | null
    : T extends Timestamp
      ? TimestampInput | null
      : T extends number
        ? number | string | null
        : T extends boolean
          ? boolean | string | null
          : T | null;

/**
 * An entity value host object. Fields are read and written as properties (`value.statusId`, `value["statusId"]`) and
 * with `get` and `set`, as the QuickJS host exposes them. A method wins over a field of the same name. A relationship
 * name also reads as a property, the related value or list; use `findRelated`/`findRelatedOne` for a typed result.
 */
type EntityValue<N extends string = string> = EntityValueMethods<N> & MoquiFieldProperties<N>;
/** The fields of entity `N` as properties, except a name a method has. */
type MoquiFieldProperties<N extends string> = {
    [K in Exclude<MoquiFieldName<N>, keyof EntityValueMethods<N>>]: MoquiFields<N>[K] | null;
};
interface EntityValueMethods<N extends string = string> {
    get<K extends MoquiFieldName<N>>(field: K): MoquiFields<N>[K] | null;
    set<K extends MoquiFieldName<N>>(field: K, value: MoquiFieldInput<MoquiFields<N>[K]>): void;
    setFields(fields: MoquiMap, setIfEmpty?: boolean, namePrefix?: string | null, pks?: boolean | null): void;
    setSequencedIdPrimary(): void;
    setSequencedIdSecondary(): void;
    create(): void;
    createOrUpdate(): void;
    update(): void;
    delete(): void;
    deleteRelated(relationship: keyof MoquiRel<N> & string): void;
    findRelated<R extends keyof MoquiRel<N> & string>(
        relationship: R,
        byAndFields?: MoquiMap | null,
        orderBy?: string[] | null,
        useCache?: boolean | null,
        forUpdate?: boolean | null,
    ): EntityList<MoquiRelated<N, R>>;
    /** The primary key fields of the value. */
    getPrimaryKeys(): { [K in MoquiPk<N> & MoquiFieldName<N>]?: MoquiFields<N>[K] };
    findRelatedOne<R extends keyof MoquiRel<N> & string>(
        relationship: R,
        useCache?: boolean | null,
        forUpdate?: boolean | null,
    ): EntityValue<MoquiRelated<N, R>> | null;
}

/** An entity list host object: index, length, iterable, `map` and `filter`. */
interface EntityList<N extends string = string> extends Iterable<EntityValue<N>> {
    readonly [index: number]: EntityValue<N>;
    readonly length: number;
    readonly pageIndex: number;
    readonly pageSize: number;
    get(index: number): EntityValue<N> | null;
    size(): number;
    getPartialList(offset: number, size: number, countLimit?: boolean): EntityList<N>;
    filterByDate(
        fromField?: string | null,
        thruField?: string | null,
        moment?: TimestampInput | null,
        ignoreIfEmpty?: boolean,
    ): EntityList<N>;
    filterByLimit(offset: number, limit: number): EntityList<N>;
    map<T>(callback: (value: EntityValue<N>, index: number) => T): T[];
    filter(callback: (value: EntityValue<N>, index: number) => boolean): EntityValue<N>[];
}

/** A condition made by `ec.entity.conditionFactory`. */
interface EntityCondition {
    readonly __moquiCondition: never;
}

/** A find builder. Each chain method returns the builder; a builder runs once. */
interface EntityFind<N extends string = string> {
    condition(field: MoquiFieldName<N>, value: unknown): EntityFind<N>;
    condition(fields: MoquiMap | null): EntityFind<N>;
    condition(condition: EntityCondition | null): EntityFind<N>;
    havingCondition(condition: EntityCondition | null): EntityFind<N>;
    searchFormMap(inputs: MoquiMap, defaultParameters?: MoquiMap | null, skipFields?: string | null, ...rest: unknown[]): EntityFind<N>;
    requireSearchFormParameters(require?: boolean): EntityFind<N>;
    useCache(flag?: boolean | null): EntityFind<N>;
    forUpdate(flag?: boolean | null): EntityFind<N>;
    useClone(flag?: boolean | null): EntityFind<N>;
    distinct(flag?: boolean | null): EntityFind<N>;
    disableAuthz(): EntityFind<N>;
    offset(offset: number | null): EntityFind<N>;
    limit(limit: number | null): EntityFind<N>;
    selectField(field: string): EntityFind<N>;
    /** Comma separated fields; `-` descending, `^` case-insensitive. */
    orderBy(fields: string): EntityFind<N>;
    copy(): EntityFind<N>;
    one(): EntityValue<N> | null;
    list(): EntityList<N>;
    iterator(): EntityList<N>;
    count(): number;
    /** How many rows were deleted. */
    deleteAll(): number;
    getLimit(): number | null;
    getOffset(): number | null;
    getPageIndex(): number;
    getPageSize(): number;
}

// ---- services ----

/** Filled by the generated `services.d.ts`: input and output maps by service name. */
interface MoquiServiceIn {}
interface MoquiServiceOut {}

type MoquiAutoVerb = "create" | "store" | "update" | "delete";
type MoquiAutoInput<N extends string> = { [K in MoquiFieldName<N>]?: MoquiFieldInput<MoquiFields<N>[K]> } & {
    [R in keyof MoquiRel<N> & string]?: MoquiMap | MoquiMap[];
};
/**
 * The implicit entity-auto verbs. The validation of an implicit service declares no parameter; the runner needs the
 * primary key for `update` and `delete`, so those are required here. Nested related records go under a relationship name.
 */
type MoquiEntityAutoIn<V extends MoquiAutoVerb, N extends string> = V extends "update" | "delete"
    ? { [K in MoquiPk<N>]: MoquiFieldInput<MoquiFields<N>[K & MoquiFieldName<N>]> } & MoquiAutoInput<N>
    : MoquiAutoInput<N>;
/** What an entity-auto service returns: the primary key of the record. */
type MoquiEntityAutoOut<N extends string> = { [K in MoquiPk<N> & MoquiFieldName<N>]?: MoquiFields<N>[K] };

type MoquiAllEntityAutoIn = {
    [E in keyof MoquiEntityMap & string as `create#${E}`]: MoquiEntityAutoIn<"create", E>;
} & {
    [E in keyof MoquiEntityMap & string as `store#${E}`]: MoquiEntityAutoIn<"store", E>;
} & {
    [E in keyof MoquiEntityMap & string as `update#${E}`]: MoquiEntityAutoIn<"update", E>;
} & {
    [E in keyof MoquiEntityMap & string as `delete#${E}`]: MoquiEntityAutoIn<"delete", E>;
};

/** The input map of service `S`: a defined service, an implicit entity-auto verb, or a plain map for a name not loaded. */
type MoquiIn<S extends string> = S extends keyof MoquiServiceIn
    ? MoquiServiceIn[S]
    : S extends `${infer V}#${infer E}`
      ? V extends MoquiAutoVerb
          ? E extends keyof MoquiEntityMap
              ? MoquiEntityAutoIn<V, E>
              : MoquiMap
          : MoquiMap
      : S extends `${infer V}-${infer E}`
        ? V extends MoquiAutoVerb
            ? E extends keyof MoquiEntityMap
                ? MoquiEntityAutoIn<V, E>
                : MoquiMap
            : MoquiMap
        : MoquiMap;
type MoquiOut<S extends string> = S extends keyof MoquiServiceOut
    ? MoquiServiceOut[S]
    : S extends `${infer V}#${infer E}`
      ? V extends MoquiAutoVerb
          ? MoquiEntityAutoOut<E>
          : MoquiMap
      : MoquiMap;
/** Service names an editor completes. Any other string is accepted with a plain map. */
type MoquiServiceName = keyof MoquiServiceIn | keyof MoquiAllEntityAutoIn | (string & {});

interface ServiceCallOptions<Self> {
    disableAuthz(): Self;
    multi(flag?: boolean): Self;
    ignoreTransaction(flag?: boolean): Self;
    requireNewTransaction(flag?: boolean): Self;
    transactionTimeout(seconds: number | null): Self;
    useTransactionCache(flag?: boolean): Self;
}

interface ServiceCallSync extends ServiceCallOptions<ServiceCallSync> {
    name<S extends MoquiServiceName & string>(name: S): ServiceCallSyncNamed<S>;
}

interface ServiceCallSyncNamed<S extends string> extends ServiceCallOptions<ServiceCallSyncNamed<S>> {
    parameters(parameters: MoquiIn<S>): ServiceCallSyncNamed<S>;
    parameter<K extends keyof MoquiIn<S> & string>(name: K, value: MoquiIn<S>[K]): ServiceCallSyncNamed<S>;
    call(): MoquiOut<S>;
}

interface ServiceCallAsync extends ServiceCallOptions<ServiceCallAsync> {
    name<S extends MoquiServiceName & string>(name: S): ServiceCallAsyncNamed<S>;
}

interface ServiceCallAsyncNamed<S extends string> extends ServiceCallOptions<ServiceCallAsyncNamed<S>> {
    parameters(parameters: MoquiIn<S>): ServiceCallAsyncNamed<S>;
    parameter<K extends keyof MoquiIn<S> & string>(name: K, value: MoquiIn<S>[K]): ServiceCallAsyncNamed<S>;
    call(): void;
    callFuture(): ServiceResultFuture<S>;
}

interface ServiceResultFuture<S extends string> {
    isDone(): boolean;
    get(timeoutMillis?: number): MoquiOut<S>;
}

// ---- server ec: the facade surface ----

interface MoquiEntityFacade {
    find<N extends (keyof MoquiEntityMap & string) | (string & {})>(entityName: N): EntityFind<N>;
    makeValue<N extends (keyof MoquiEntityMap & string) | (string & {})>(entityName: N): EntityValue<N>;
    getDataDocuments(...args: unknown[]): MoquiMap[];
    readonly conditionFactory: {
        makeCondition(field: string, operator: string, value: unknown, ignoreCase?: boolean): EntityCondition;
        makeCondition(conditions: EntityCondition[], joinOperator?: string): EntityCondition;
        makeCondition(fields: MoquiMap): EntityCondition;
        makeConditionDate(
            fromField: string | null,
            thruField: string | null,
            moment: TimestampInput | null,
            ignoreIfEmpty?: boolean,
            ignore?: string | boolean | null,
        ): EntityCondition | null;
        makeActionConditionDirect(
            field: string,
            operator: string | null,
            value: unknown,
            toField: string | null,
            ignoreCase: boolean | null,
            ignoreIfEmpty: boolean | null,
            orNull: boolean | null,
            not: boolean | null,
            ignore: string | boolean | null,
        ): EntityCondition | null;
    };
}

interface MoquiResourceFacade {
    /** Localizes text shorter than 256 characters, then expands `${}` against the context and `map`. */
    expand(text: string, location?: string | null, map?: MoquiMap | null): string;
    expandNoL10n(text: string, location?: string | null, map?: MoquiMap | null): string;
    expression(expression: string, location?: string | null, map?: MoquiMap | null): unknown;
    condition(expression: string, location?: string | null, map?: MoquiMap | null): boolean;
    script(location: string): void;
    template(location: string, data?: MoquiMap | null): string;
    /** PDF bytes of an `html-print` (old name `xsl-fo`) document. `xslt` must be null and `out` is ignored. Throws when PDF is not available. */
    xslFoTransform(document: string, xslt?: null, out?: unknown, contentType?: string | null): Uint8Array;
}

interface MoquiL10nFacade {
    localize(text: string): string;
    format(value: unknown, pattern?: string | null): string;
    formatCurrency(amount: unknown, uomId?: string | null, fractionDigits?: number | null): string;
    formatCurrencyNoSymbol(amount: unknown, uomId?: string | null, fractionDigits?: number | null): string;
    parseTimestamp(text: string, format?: string | null): Timestamp | null;
}

interface MoquiMessageFacade {
    addError(message: string): void;
    addMessage(message: string, type?: string): void;
    addPublic(message: string, type?: string): void;
    hasError(): boolean;
    getErrorsString(): string;
    clearErrors(): void;
    readonly errors: string[];
}

interface MoquiLoggerFacade {
    log(level: string | number, message: string, thrown?: unknown): void;
    warn(message: string): void;
}

interface MoquiUserFacade {
    readonly userId: string | null;
    readonly username: string | null;
    readonly locale: string;
    getLocale(): string;
    readonly timeZone: string;
    getTimeZone(): string;
    /** The signed-in `UserAccount` without its credential columns. */
    readonly userAccount: MoquiMap | null;
    readonly nowTimestamp: Timestamp;
    readonly passwordRules: { minLength: number; minDigits: number; minOthers: number };
    getPreference(key: string): string | null;
    loginUser(username: string, password: string): boolean;
    internalLoginUser(username: string): boolean;
    logoutUser(): boolean;
}

/** An uploaded file. A script cannot hold the bytes. */
interface MoquiUploadedFile {
    readonly name: string;
    readonly fileName: string;
    readonly fieldName: string;
    readonly contentType: string;
    readonly size: number;
    getString(): string;
    /** `dbresource://` only. */
    write(location: string): void;
    isInMemory(): boolean;
}

interface MoquiPageMessage {
    message: string;
    typeString: string;
}

interface MoquiWebFacade {
    readonly parameters: MoquiMap;
    readonly requestParameters: MoquiMap;
    readonly multiPartParameters: MoquiMap;
    readonly sessionAttributes: MoquiMap;
    readonly sessionToken: string;
    readonly savedMessages: string[];
    readonly savedErrors: string[];
    readonly savedValidationErrors: unknown[];
    readonly errorParameters: MoquiMap | null;
    sendJsonResponse(value: unknown): void;
    sendJsonError(status: number, message: string, error?: unknown): void;
    sendTextResponse(text: string): void;
    /** The bytes at a resource location, inline or (the default) as an attachment. 404 when nothing is there. */
    sendResourceResponse(location: string, inline?: boolean): void;
    saveScreenLastInfo(path?: string | null, parameters?: MoquiMap | null): void;
    getPageMessages(): MoquiPageMessage[];
    getScreenHistory(): { name: string; url: string; urlNoParams: string }[];
    getHostName(withPort?: boolean): string;
    getResourceDistinctValue(): string;
    /** The base URL. The arguments are ignored. */
    getWebappRootUrl(...ignored: unknown[]): string;
}

interface MoquiScreenRenderOptions {
    screenPath: string;
    renderMode?: string;
    rootScreen?: string;
    baseLinkUrl?: string;
    lastStandalone?: string;
    parameters?: MoquiMap;
}

interface MoquiScreenRenderBuilder {
    webappName(name: string): MoquiScreenRenderBuilder;
    rootScreen(location: string): MoquiScreenRenderBuilder;
    /** Ignored. */
    rootScreenFromHost(host: string): MoquiScreenRenderBuilder;
    screenPath(path: string): MoquiScreenRenderBuilder;
    renderMode(mode: string): MoquiScreenRenderBuilder;
    baseLinkUrl(url: string): MoquiScreenRenderBuilder;
    lastStandalone(value: string): MoquiScreenRenderBuilder;
    parameters(parameters: MoquiMap): MoquiScreenRenderBuilder;
    render(): string;
}

interface MoquiScreenFacade {
    isRenderModeValid(mode: string): boolean;
    isRenderModeText(mode: string): boolean;
    getMimeTypeByMode(mode: string): string | null;
    render(options: MoquiScreenRenderOptions): { status: number; contentType: string; text: string };
    makeRender(): MoquiScreenRenderBuilder;
}

interface MoquiEcContext {
    get(name: string): unknown;
}

/**
 * The server `ec` an action, a service script, or a script outside a render sees: the facade surface.
 * `ec.cache` and `ec.search` are outside the Server host methods list and are typed loosely.
 */
interface MoquiServerEc {
    readonly entity: MoquiEntityFacade;
    readonly service: { sync(): ServiceCallSync; async(): ServiceCallAsync };
    readonly resource: MoquiResourceFacade;
    readonly l10n: MoquiL10nFacade;
    readonly message: MoquiMessageFacade;
    readonly logger: MoquiLoggerFacade;
    readonly user: MoquiUserFacade;
    readonly web: MoquiWebFacade;
    readonly screen: MoquiScreenFacade;
    readonly context: MoquiEcContext;
    readonly factory: { readonly moquiVersion: string };
    readonly cache: { getCache(name: string): { get(key: string): unknown; put(key: string, value: unknown): void; remove(key: string): boolean; containsKey(key: string): boolean; clear(): void } };
    /**
     * The tools calls (phase 11): cache admin, the entity and service model, screen statistics, the server status,
     * the SQL runner, the data writer and loader. They are outside the Server host methods list and typed loosely.
     */
    readonly artifactExecution: { isPermitted(name: string, artifactType?: string, action?: string): boolean };
    readonly search: { [method: string]: (...args: unknown[]) => unknown };
}

// ---- server ec and sri during a screen render ----

/** `ec` while a server screen render is on the stack: the form-list keys of `ec.context` are set. */
interface MoquiRenderEc extends MoquiServerEc {
    readonly context: MoquiEcContext & { readonly formListFindId: string | null; readonly orderByField: string | null };
}

/** An element as generated screen code writes it. */
interface MoquiElement {
    name: string;
    attrs: Record<string, string>;
    text?: string;
    children?: MoquiElement[];
    parent?: MoquiElement | null;
}

interface MoquiScreenUrl {
    readonly path: string;
    readonly url: string;
    readonly pathWithParams: string;
    readonly readOnly: boolean;
    getPath(): string;
    getUrl(): string;
    getPathWithParams(): string;
    getUrlWithParams(extension?: string): string;
    getMinimalPathUrl(): string;
    isReadOnly(): boolean;
    isScreenUrl(): boolean;
    isTransition(): boolean;
    getTransitionMethod(): string;
    cloneUrlInstance(): MoquiScreenUrl;
    addParameter(name: string, value: unknown): MoquiScreenUrl;
    addParameters(parameters: MoquiMap): MoquiScreenUrl;
    removeParameter(name: string): MoquiScreenUrl;
    getParameterMap(): MoquiMap;
    getPassThroughParameterMap(): MoquiMap;
}

interface MoquiScreenDefInfo {
    readonly location: string;
    readonly screenName: string;
    getLocation(): string;
    getScreenName(): string;
    getDefaultMenuName(): string;
}

interface MoquiMenuItem {
    name: string;
    title: string;
    url: string;
    disabled: boolean;
    inPath: boolean;
    image: string | null;
    imageType: string | null;
    hasParameters: boolean;
}

interface MoquiFormListInfo {
    id: number;
    count: number;
}

/** `sri`: only while a server screen render is on the stack. Absent in actions that are not a render, and in the browser. */
interface MoquiScreenRenderer {
    readonly activeInCurrentMenu: boolean;
    write(text: string): void;
    getRenderMode(): string;
    doBoundaryComments(): boolean;
    encodeHtml(text: unknown): string;
    getActiveScreenDef(): MoquiScreenDefInfo;
    getScreenUrlInfo(): { targetScreen: MoquiScreenDefInfo; parentScreen: MoquiScreenDefInfo | null; extraPathNameList: string[] };
    sendRedirectAndStopRender(url: string): void;
    renderSection(name: string): string;
    renderSectionInclude(nameOrLocation: string): string;
    renderSubscreen(): string;
    renderIncludeScreen(location: string, shareScope?: string | boolean | null): string;
    renderText(location: string, template?: string | boolean | null): string;
    renderInlineText(body: string): string;
    getSectionIncludedNode(name: string): MoquiElement | null;
    once(key: string): boolean;
    pushContext(): void;
    popContext(): void;
    setInContext(element: MoquiElement): void;
    pushSingleFormMapContext(map: string | MoquiMap | null): void;
    getSingleFormMap(map: string | MoquiMap | null): MoquiMap;
    getFieldValue(field: MoquiElement, defaultValue?: string | null): unknown;
    getFieldValueString(widgetOrField: MoquiElement, defaultValue?: string | null, format?: string | null, type?: string | null): string;
    getFieldValuePlainString(field: MoquiElement, defaultValue?: string | null): string;
    getFieldValueClass(field: MoquiElement): string;
    getNamedValuePlain(name: string, form: MoquiElement): string;
    getFieldEntityValue(widget: MoquiElement): string;
    getFieldOptions(widget: MoquiElement): Map<string, string>;
    getFieldValidation(subNode: MoquiElement): { classes: string; regexp?: string; message?: string };
    getFormNode(name: string): MoquiElement;
    getFormInstance(name: string): { formNode: MoquiElement; name: string; list: boolean };
    getFormHiddenParameters(form: MoquiElement): MoquiMap;
    openFormList(name: string): MoquiFormListInfo;
    startFormListRow(info: MoquiFormListInfo, index: number): void;
    endFormListRow(): void;
    safeCloseList(info: MoquiFormListInfo): void;
    formListColumnCharWidths(...args: unknown[]): unknown;
    buildUrl(target: string): MoquiScreenUrl;
    buildUrlFromTarget(target: string): MoquiScreenUrl;
    makeUrlByType(origUrl: string, urlType: string, element?: MoquiElement | null, expandTransitionUrl?: string | boolean | null): MoquiScreenUrl;
    getScreenUrlInstance(): MoquiScreenUrl;
    getCurrentScreenUrl(): MoquiScreenUrl;
    isAnchorLink(element: MoquiElement, url: MoquiScreenUrl): boolean;
    getThemeIconClass(text: string): string | null;
    getThemeValues(resourceTypeEnumId: string): string[];
    getMenuItems(path?: string): MoquiMenuItem[];
    getMenuData(pathNames: string[]): MoquiMap[] | null;
    appendToAfterScreenWriter(text: string): void;
    appendToScriptWriter(text: string): void;
    getAfterScreenWriterText(): string;
    getScriptWriterText(): string;
    staticUrl(text: string, urlType: string): { path: string; exists: boolean; isTransition: boolean; isScreen: boolean; readOnly: boolean; method: string };
    staticCheck(json: string): void;
    staticNote(kind: string, element: string, detail: string): void;
}

// ---- statics the generated script calls ----

/**
 * Bound once per runtime and returned by the bare-name proxy when the context has no such key. This port binds no
 * method on `ObjectUtilities` yet. `ScheduledJobRunner.getCronDescription` is not added (Server host methods, slice 2).
 */
interface MoquiObjectUtilities {}

interface MoquiCollectionUtilities {
    /** Sets `{list}Count`, `{list}PageIndex`, `{list}PageSize`, `{list}PageMaxIndex`, `{list}PageRangeLow`, `{list}PageRangeHigh` on `context`. */
    paginateParameters(count: number, listName: string, context: MoquiMap): MoquiMap;
    getMapArrayListValues(list: MoquiMap[], field: string, skipNull: boolean): unknown[];
}

interface MoquiStringUtilities {
    encodeForXmlAttribute(text: unknown, addZeroWidthSpaces?: boolean): string;
    base64UrlEncode(text: string): string;
    base64UrlDecode(text: string): string;
}

interface MoquiWebUtilities {
    encodeHtml(text: unknown): string;
    encodeHtmlJsSafe(text: unknown): string;
    fieldValuesEncodeHtmlJsSafe(values: unknown): string;
    ipMatches(patterns: string, address: string): boolean;
    isSafeSinglePathSegment(segment: string): boolean;
    isSameOriginRedirect(url: string, host?: string | null): boolean;
    lastPathSegment(path: string): string;
}

// ---- the unwrapped projection ----

/**
 * `ContextJavaUtil.unwrapMap`: plain maps, a decimal as a string, a timestamp as ISO-8601 text, an entity value as its
 * field map, an entity list as a list of those maps. A find, `ec`, `sri` and functions are dropped.
 * This is the JSON, the locked-down template, and the browser shape.
 */
type MoquiUnwrap<T> = T extends Decimal
    ? string
    : T extends Timestamp
      ? Timestamp
      : T extends string | number | boolean | null | undefined
        ? T
        : T extends EntityValue<infer N>
          ? MoquiPlain<N>
          : T extends EntityList<infer N>
            ? MoquiPlain<N>[]
            : T extends EntityFind<string> | MoquiServerEc | MoquiScreenRenderer | ((...args: never[]) => unknown)
              ? never
              : T extends readonly (infer U)[]
                ? MoquiUnwrap<U>[]
                : T extends object
                  ? MoquiUnwrapMap<T>
                  : unknown;
type MoquiUnwrapMap<T> = {
    [K in keyof T as [MoquiUnwrap<T[K]>] extends [never] ? never : K]: MoquiUnwrap<T[K]>;
};
/** An entity value as plain data: its field map. */
type MoquiPlain<N extends string> = { [K in MoquiFieldName<N>]?: MoquiUnwrap<MoquiFields<N>[K]> | null };

// ---- screens ----

/** Filled by the generated `screens.d.ts`. */
interface MoquiScreenContexts {}
interface MoquiScreenPackages {}
interface MoquiFormListPackages {}

// ---- the browser host and the server-static qvt package ----

/** The restricted `ec` of a server-static qvt screen in the browser. Any other member throws there. */
interface MoquiBrowserEc {
    readonly l10n: {
        localize(text: string): string;
        format(value: unknown, pattern?: string | null): string;
        formatCurrency(amount: unknown, uomId?: string | null, fractionDigits?: number | null): string;
        formatCurrencyNoSymbol(amount: unknown, uomId?: string | null, fractionDigits?: number | null): string;
    };
    readonly user: {
        readonly userId: string | null;
        readonly username: string | null;
        readonly locale: string;
        readonly timeZone: string;
        readonly userAccount: {
            partyId?: string | null;
            emailAddress?: string | null;
            userFullName?: string | null;
            locale?: string | null;
            timeZone?: string | null;
        } | null;
        getPreference(key: string): string | null;
    };
    readonly resource: {
        /** The template carries a table of functions by expression text; the browser has no eval. */
        expand(text: string, table?: Record<string, () => unknown>): string;
        expandNoL10n(text: string, table?: Record<string, () => unknown>): string;
    };
}

/** A drop-down option the server resolved: `mqOpt_{form}_{field}_{sub}_{widget}`. */
interface MoquiOption {
    value: string;
    label: string;
}

type MoquiPageKey<L extends string> = `${L}${MoquiPageSuffix}`;
/** The plain projection with credential names removed at any depth, as `stripCredentialParameters` removes them. */
type MoquiStripped<T> = T extends readonly (infer U)[]
    ? MoquiStripped<U>[]
    : T extends string | number | boolean | null | undefined
      ? T
      : T extends object
        ? { [K in keyof T as K extends string ? (Lowercase<K> extends MoquiCredentialName ? never : K) : K]: MoquiStripped<T[K]> }
        : T;

/**
 * `GET {screen}/actions`: the context after data prep, unwrapped, credentials stripped, the lists and page keys of every
 * form-list removed (`L`), plus the `mqExtra_{form}` maps of the form-singles (`X`). Keys the generator cannot prove are unknown.
 */
type MoquiScreenPackage<C, L extends string, X> = MoquiStripped<
    MoquiUnwrapMap<{ [K in keyof C as K extends L | MoquiPageKey<L> ? never : K]: C[K] }>
> &
    X & { [name: string]: unknown };

/** `GET {screen}/actions/{formName}`: the rows, the page keys, the header values, the option lists. */
type MoquiFormListPackage<L extends string, R, X> = { [K in L]: MoquiStripped<R>[] } & {
    [K in MoquiPageKey<L>]?: number;
} & {
    orderByField?: string;
    pageIndex?: number | string;
    pageSize?: number | string;
    formListFindId?: string;
} & X;

/** The scope a server-static template expression sees: the package and the browser `ec`. */
type MoquiStaticScope<P> = P & { readonly ec: MoquiBrowserEc };
