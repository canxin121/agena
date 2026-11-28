function schemaObject(schema) {
    if (!schema || typeof schema !== 'object' || Array.isArray(schema)) {
        throw new TypeError('Cordis tool schema must be an object');
    }
    return schema;
}

function properties(spec) {
    const required = [];
    const entries = Object.entries(schemaObject(spec)).map(([name, child]) => {
        const { required: present, ...schema } = schemaObject(child);
        if (present !== undefined && present !== true) {
            throw new TypeError(`invalid required flag for Cordis tool field ${name}`);
        }
        if (present) required.push(name);
        return [name, valueSchema(schema)];
    });
    return { properties: Object.fromEntries(entries), ...(required.length && { required }) };
}

function valueSchema(spec) {
    const schema = { ...schemaObject(spec) };
    if (schema.type === 'json') delete schema.type;
    if (schema.properties !== undefined) Object.assign(schema, properties(schema.properties));
    if (schema.items !== undefined) schema.items = valueSchema(schema.items);
    if (schema.oneOf !== undefined) schema.oneOf = schema.oneOf.map(valueSchema);
    return schema;
}

export function defineTool(options) {
    return {
        ...options,
        parameters: { type: 'object', additionalProperties: false, ...properties(options.parameters) },
        output: { ...options.output, schema: valueSchema(options.output?.schema) },
    };
}
