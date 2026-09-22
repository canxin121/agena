(() => {
    const clean = value => String(value || '').replace(/\s+/g, ' ').trim();
    const clip = (text, limit) => {
        let end = Math.min(text.length, limit);
        if (end > 0 && end < text.length && text.charCodeAt(end - 1) >= 0xD800 && text.charCodeAt(end - 1) <= 0xDBFF && text.charCodeAt(end) >= 0xDC00 && text.charCodeAt(end) <= 0xDFFF) end -= 1;
        return text.slice(0, end);
    };
    const selector = 'a,button,input,textarea,select,[role],[contenteditable="true"]';
    const editable = 'input,textarea,select,[contenteditable="true"],[data-sensitive]';
    const label = el => {
        const explicit = el.getAttribute('aria-label') || el.getAttribute('placeholder') || el.getAttribute('title') || el.getAttribute('name');
        // Ancestor role containers can include textarea/contenteditable text
        // in innerText too. Walk bounded text and omit sensitive descendants.
        if (el.closest(editable)) return clean(explicit);
        const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
        const parts = [];
        let visited = 0, length = 0;
        while (walker.nextNode() && ++visited <= 1024 && length < 300) {
            const node = walker.currentNode, parent = node.parentElement;
            if (!parent || parent.closest(`${editable},script,style,noscript`) || !parent.getClientRects().length || getComputedStyle(parent).visibility === 'hidden') continue;
            const text = clip(clean(node.nodeValue), 300 - length);
            if (text) { parts.push(text); length += text.length + 1; }
        }
        return clean(parts.join(' ') || explicit);
    };
    const signature = el => JSON.stringify([el.tagName, el.getAttribute('role'), el.getAttribute('type'), el.id, el.getAttribute('name'), el.getAttribute('href'), label(el)]);
    const nodes = Array.from(document.querySelectorAll(selector)).filter(el =>
        el.isConnected && el.getAttribute('type') !== 'hidden' && el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden'
    ).slice(0, 200);
    const snapshotId = __AGENA_SNAPSHOT_ID__;
    Object.defineProperty(globalThis, Symbol.for('agena.browser.snapshot'), {
        configurable: true,
        value: {id: snapshotId, document, url: location.href, nodes, signatures: nodes.map(signature), signature}
    });
    const elements = nodes.map((el, index) => ({
        ref: index, tag: el.tagName.toLowerCase(), role: el.getAttribute('role'),
        type: el.getAttribute('type'), name: el.getAttribute('name'), id: el.id || null,
        text: clip(label(el), 300), disabled: Boolean(el.disabled),
        value_omitted: el.matches(editable)
    }));
    // Walk bounded visible text, omitting editable subtrees instead of reading
    // body.innerText, which can include textarea/contenteditable secrets.
    const chunks = [];
    let length = 0, visited = 0, truncated = false;
    const walker = document.body && document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    while (walker && walker.nextNode()) {
        if (++visited > 100000 || length >= 50000) { truncated = true; break; }
        const node = walker.currentNode, parent = node.parentElement;
        if (!parent || parent.closest(`${editable},script,style,noscript`) || !parent.getClientRects().length || getComputedStyle(parent).visibility === 'hidden') continue;
        const text = clip(clean(node.nodeValue), 50000 - length);
        if (text) { chunks.push(text); length += text.length + 1; }
    }
    return {snapshot_id: snapshotId, url: location.href, title: document.title,
        ready_state: document.readyState, text: chunks.join(' '), text_truncated: truncated,
        elements, form_values_omitted: true};
})()
