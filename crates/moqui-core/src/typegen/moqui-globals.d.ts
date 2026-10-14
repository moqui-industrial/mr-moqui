// The names a server script sees without declaring them (plans/ContextObjects.md). Declarations only.
// A bare name on the context stack is not declared here: read it through `context`, or declare it in the script.

/** The execution context: the facade surface. During a screen render it is also `MoquiRenderEc`. */
declare const ec: MoquiServerEc;
/** The context stack. A screen script can narrow it: `context as MoquiScreenContexts["component://..."]`. */
declare const context: MoquiContext;
/** The result map of a service's actions. */
declare let result: MoquiMap;
/** The screen renderer. Its methods work only while a server screen render is on the stack. */
declare const sri: MoquiScreenRenderer;

declare const ObjectUtilities: MoquiObjectUtilities;
declare const CollectionUtilities: MoquiCollectionUtilities;
declare const StringUtilities: MoquiStringUtilities;
declare const WebUtilities: MoquiWebUtilities;
declare const System: { getProperty(name: string): string | null };

/** The compare of `<if-compare>` and `<econdition>`: `operator` is a Moqui operator name such as `equals`. */
declare function compare(
    field: unknown,
    operator: string,
    value: unknown,
    toField?: unknown,
    format?: string | null,
    type?: string | null,
): boolean;
/** `ObjectUtilities.basicConvert`: `type` is a Java type name such as `BigDecimal`. */
declare function basicConvert(value: unknown, type: string): unknown;
declare function orderMapList(list: unknown[] | null, fields: string[]): void;
declare function filterMapList(list: unknown[] | null, values: MoquiMap): void;
declare function filterMapListByDate(
    list: unknown[] | null,
    fromField: string,
    thruField: string,
    moment?: TimestampInput | null,
    ignoreIfEmpty?: boolean,
): void;
