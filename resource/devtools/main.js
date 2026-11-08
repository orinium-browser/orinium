(function () {
  "use strict";

  var statusEl = document.getElementById("status");
  var targetEl = document.getElementById("target");
  var refreshEl = document.getElementById("refresh");
  var treeEl = document.getElementById("tree");
  var attributesEl = document.getElementById("attributes");
  var stylesEl = document.getElementById("styles");
  var computedEl = document.getElementById("computed");
  var layoutEl = document.getElementById("layout");
  var detailTabsEl = document.getElementById("detail-tabs");
  var traceEventsEl = document.getElementById("trace-events");
  var traceDetailEl = document.getElementById("trace-detail");
  var traceStatusEl = document.getElementById("trace-status");
  var traceToggleEl = document.getElementById("trace-toggle");
  var traceActiveEl = document.getElementById("trace-active");
  var consoleOutputEl = document.getElementById("console-output");
  var consoleInputEl = document.getElementById("console-input");
  var consoleRequestId = 0;
  var consoleLastTraceSeq = 0;
  var sourceListEl = document.getElementById("source-list");
  var sourceDetailEl = document.getElementById("source-detail");
  var sourceEntries = {};
  var traceEnabled = true;
  var traceLastSeq = 0;
  var emptyNotes = {};
  var emptyNoteEls = document.getElementsByClassName("view-empty");
  for (var noteIndex = 0; noteIndex < emptyNoteEls.length; noteIndex += 1) {
    emptyNotes[emptyNoteEls[noteIndex].getAttribute("data-for")] =
      emptyNoteEls[noteIndex];
  }

  // Latest serialized tree plus UI state keyed by inspector dom ids, kept
  // across refreshes so expansion and selection survive updates.
  var tree = null;
  var expanded = {};
  var selectedId = null;
  var domVersion = null;
  var layoutVersion = null;

  function call(method, params) {
    return __orinium_devtools(method, JSON.stringify(params || {})).then(
      function (json) {
        var envelope = JSON.parse(json);
        if (!envelope.ok) {
          throw new Error(envelope.error || "request failed");
        }
        return envelope.data;
      },
    );
  }

  function setStatus(state) {
    statusEl.setAttribute("data-state", state);
    statusEl.textContent =
      state === "connected" ? "connected" : statusEl.textContent;
    if (state !== "connected") {
      statusEl.textContent = state;
    }
  }

  function findById(node, id) {
    if (!node || node.id === id) {
      return node;
    }
    var children = node.children || [];
    for (var i = 0; i < children.length; i += 1) {
      var found = findById(children[i], id);
      if (found) {
        return found;
      }
    }
    return null;
  }

  // The synthetic roots ("document") render children just like elements do;
  // every other node type is displayed as a leaf preview.
  function hasChildren(node) {
    return !!(
      node &&
      (node.type === "element" || node.type === "document") &&
      node.children &&
      node.children.length
    );
  }

  function describe(node) {
    if (node.type === "text") {
      var preview = node.text.replace(/\s+/g, " ").trim();
      if (preview.length > 40) {
        preview = preview.slice(0, 40) + "\u2026";
      }
      if (!preview) {
        preview = "(whitespace)";
      }
      return '"' + preview + '"';
    }
    if (node.type === "comment") {
      return "<!--\u2026-->";
    }
    if (node.type === "doctype") {
      return "<!doctype>";
    }
    if (node.type === "document") {
      return "#document";
    }
    var label = "<" + node.tag;
    var attributes = node.attributes || [];
    for (var i = 0; i < attributes.length; i += 1) {
      if (attributes[i][0] === "id" || attributes[i][0] === "class") {
        label += " " + attributes[i][0] + '="' + attributes[i][1] + '"';
      }
    }
    return label + ">";
  }

  function setEmpty(view, empty) {
    if (emptyNotes[view]) {
      emptyNotes[view].style.display = empty ? "" : "none";
    }
  }

  function clearDetails() {
    attributesEl.textContent = "";
    stylesEl.textContent = "";
    computedEl.textContent = "";
    layoutEl.textContent = "";
    targetEl.textContent = "";
    setEmpty("attributes", true);
    setEmpty("styles", true);
    setEmpty("computed", true);
    setEmpty("layout", true);
  }

  function select(node) {
    selectedId = node.id;
    renderTree();
    clearDetails();
    setEmpty("attributes", false);
    setEmpty("styles", false);
    setEmpty("computed", false);
    setEmpty("layout", false);
    showAttributes(node.id);
    showStyles(node.id);
    showComputed(node.id);
    showLayout(node.id);
  }

  function buildRow(node) {
    var row = document.createElement("div");
    row.setAttribute(
      "class",
      "row" + (node.id === selectedId ? " selected" : ""),
    );

    var caret = document.createElement("span");
    caret.setAttribute("class", "caret");
    if (hasChildren(node)) {
      caret.textContent = expanded[node.id] ? "\u25be" : "\u25b8";
      caret.onclick = function (event) {
        event.stopPropagation ? event.stopPropagation() : null;
        expanded[node.id] = !expanded[node.id];
        renderTree();
      };
    } else {
      caret.textContent = " ";
    }
    row.appendChild(caret);

    var label = document.createElement("span");
    label.textContent = describe(node);
    if (node.type === "element") {
      label.setAttribute("class", "tag");
    } else if (node.type === "text") {
      label.setAttribute("class", "text-preview");
    } else {
      label.setAttribute(
        "class",
        node.type === "comment" ? "comment-preview" : "doctype-preview",
      );
    }
    row.appendChild(label);

    row.onclick = function () {
      if (node.type !== "element") {
        return;
      }
      select(node);
    };

    return row;
  }

  function buildSubtree(node, container, depth) {
    container.appendChild(buildRow(node));
    if (!hasChildren(node) || !(expanded[node.id] || depth < 2)) {
      return;
    }
    var childrenBox = document.createElement("div");
    childrenBox.setAttribute("class", "children");
    var children = node.children;
    for (var i = 0; i < children.length; i += 1) {
      buildSubtree(children[i], childrenBox, depth + 1);
    }
    container.appendChild(childrenBox);
  }

  function renderTree() {
    treeEl.textContent = "";
    if (!tree) {
      treeEl.textContent = "No document loaded.";
      return;
    }
    buildSubtree(tree, treeEl, 0);
  }

  function showAttributes(domId) {
    call("getAttributes", { domId: domId })
      .then(function (data) {
        if (selectedId !== domId) {
          return;
        }
        attributesEl.textContent = "";

        var head = document.createElement("tr");
        var nameHeader = document.createElement("th");
        nameHeader.textContent = "Attribute";
        var valueHeader = document.createElement("th");
        valueHeader.textContent = "Value";
        head.appendChild(nameHeader);
        head.appendChild(valueHeader);
        attributesEl.appendChild(head);

        targetEl.textContent = "<" + data.tag + ">";
        var attributes = data.attributes || [];
        for (var i = 0; i < attributes.length; i += 1) {
          var tr = document.createElement("tr");

          var nameCell = document.createElement("td");
          nameCell.setAttribute("class", "attr-name");
          nameCell.textContent = attributes[i][0];
          tr.appendChild(nameCell);

          var valueCell = document.createElement("td");
          valueCell.setAttribute("class", "attr-value");
          valueCell.textContent =
            attributes[i].length > 1 ? attributes[i][1] : "";
          tr.appendChild(valueCell);

          attributesEl.appendChild(tr);
        }
      })
      .catch(function () {
        // The element may have disappeared between selection and lookup.
      });
  }

  function appendDeclaration(declBox, declaration) {
    var line = document.createElement("div");
    line.setAttribute(
      "class",
      "decl" + (declaration.applied ? "" : " overridden"),
    );

    var nameSpan = document.createElement("span");
    nameSpan.setAttribute("class", "decl-name");
    nameSpan.textContent = declaration.name;
    line.appendChild(nameSpan);

    var valueSpan = document.createElement("span");
    valueSpan.setAttribute("class", "decl-value");
    valueSpan.textContent = ": " + declaration.value;
    line.appendChild(valueSpan);

    if (declaration.important) {
      var importantSpan = document.createElement("span");
      importantSpan.setAttribute("class", "decl-important");
      importantSpan.textContent = "!important";
      line.appendChild(importantSpan);
    }

    declBox.appendChild(line);
  }

  function showStyles(domId) {
    call("getMatchedRules", { domId: domId })
      .then(function (data) {
        if (selectedId !== domId) {
          return;
        }
        stylesEl.textContent = "";

        var rules = data.rules || [];
        for (var i = 0; i < rules.length; i += 1) {
          var rule = rules[i];

          var box = document.createElement("div");
          box.setAttribute(
            "class",
            "rule" + (rule.inline ? " inline-rule" : ""),
          );

          var header = document.createElement("div");
          header.setAttribute("class", "rule-header");

          var selector = document.createElement("span");
          selector.setAttribute("class", "rule-selector");
          selector.textContent = rule.selector;
          header.appendChild(selector);

          if (rule.origin !== "author") {
            var originBadge = document.createElement("span");
            originBadge.setAttribute("class", "rule-origin");
            originBadge.textContent = rule.origin;
            header.appendChild(originBadge);
          }
          box.appendChild(header);

          var declarations = rule.declarations || [];
          for (var j = 0; j < declarations.length; j += 1) {
            appendDeclaration(box, declarations[j]);
          }

          stylesEl.appendChild(box);
        }
      })
      .catch(function () {
        setEmpty("styles", true);
      });
  }

  function showComputed(domId) {
    call("getComputedStyle", { domId: domId })
      .then(function (data) {
        if (selectedId !== domId) {
          return;
        }
        computedEl.textContent = "";

        var properties = data.properties || [];
        for (var i = 0; i < properties.length; i += 1) {
          var tr = document.createElement("tr");

          var nameCell = document.createElement("td");
          nameCell.setAttribute("class", "attr-name");
          nameCell.textContent = properties[i].name;
          tr.appendChild(nameCell);

          var valueCell = document.createElement("td");
          valueCell.setAttribute("class", "attr-value");
          valueCell.textContent = properties[i].value;
          tr.appendChild(valueCell);

          computedEl.appendChild(tr);
        }
      })
      .catch(function () {
        setEmpty("computed", true);
      });
  }

  function appendRing(parent, className, label, value) {
    var ring = document.createElement("div");
    ring.setAttribute("class", "bm-ring " + className);

    var top = document.createElement("div");
    top.setAttribute("class", "bm-top bm-label-row");
    top.textContent = label;
    ring.appendChild(top);

    var middle = document.createElement("div");
    middle.setAttribute("class", "bm-middle");
    var left = document.createElement("span");
    left.setAttribute("class", "bm-side");
    left.textContent = value[3];
    middle.appendChild(left);

    var inner = document.createElement("div");
    inner.setAttribute("class", "bm-inner");
    if (value.length === 2) {
      inner.textContent = value[0] + " \u00d7 " + value[1];
    }
    middle.appendChild(inner);

    var right = document.createElement("span");
    right.setAttribute("class", "bm-side");
    right.textContent = value[1];
    middle.appendChild(right);
    ring.appendChild(middle);

    var bottom = document.createElement("div");
    bottom.setAttribute("class", "bm-bottom bm-label-row");
    bottom.textContent = value[2];
    ring.appendChild(bottom);

    parent.appendChild(ring);
  }

  function appendLayoutRow(table, name, value) {
    var tr = document.createElement("tr");

    var nameCell = document.createElement("td");
    nameCell.setAttribute("class", "attr-name");
    nameCell.textContent = name;
    tr.appendChild(nameCell);

    var valueCell = document.createElement("td");
    valueCell.setAttribute("class", "attr-value");
    valueCell.textContent = value;
    tr.appendChild(valueCell);

    table.appendChild(tr);
  }

  function renderLayout(model, info) {
    layoutEl.textContent = "";

    // Nest content inside padding inside border, with the margin ring
    // outermost — mirroring the DevTools box-model diagram.
    var marginBox = document.createElement("div");
    marginBox.setAttribute("class", "bm-margin");
    appendRing(
      marginBox,
      "bm-margin-ring",
      "margin",
      model.margin || ["-", "-", "-", "-"],
    );

    var rings = marginBox.getElementsByClassName("bm-inner");
    var borderInner = rings[0];
    appendRing(borderInner, "bm-border-ring", "border", model.border || []);
    var paddingInner = borderInner.getElementsByClassName("bm-inner")[0];
    appendRing(paddingInner, "bm-padding-ring", "padding", model.padding || []);
    var content = paddingInner.getElementsByClassName("bm-inner")[0];
    content.setAttribute("class", "bm-inner bm-content");
    var contentSize = model.content || [];
    if (contentSize.length === 2) {
      content.textContent = contentSize[0] + " \u00d7 " + contentSize[1];
    }

    layoutEl.appendChild(marginBox);

    var info_ = info || {};
    var position = model.position || [];
    var table = document.createElement("table");
    table.setAttribute("class", "layout-info");
    if (position.length === 2) {
      appendLayoutRow(table, "position", position[0] + ", " + position[1]);
    }
    appendLayoutRow(table, "display", info_.display || "-");
    appendLayoutRow(table, "positioning", info_.position || "-");
    appendLayoutRow(table, "width", info_.width || "-");
    appendLayoutRow(table, "height", info_.height || "-");
    appendLayoutRow(
      table,
      "children",
      typeof info_.children === "number" ? String(info_.children) : "-",
    );
    if (info_.scroll && info_.scroll.length === 2) {
      appendLayoutRow(
        table,
        "scroll",
        info_.scroll[0] + ", " + info_.scroll[1],
      );
    }
    layoutEl.appendChild(table);
  }

  function showLayout(domId) {
    call("getBoxModel", { domId: domId })
      .then(function (boxData) {
        return call("getLayoutInfo", { domId: domId }).then(
          function (infoData) {
            return [boxData, infoData];
          },
        );
      })
      .then(function (results) {
        if (selectedId !== domId) {
          return;
        }
        renderLayout(results[0].model || {}, results[1].info || {});
      })
      .catch(function () {
        setEmpty("layout", true);
      });
  }

  function activateTab(tabName) {
    var tabs = detailTabsEl.getElementsByClassName("detail-tab");
    for (var i = 0; i < tabs.length; i += 1) {
      var active = tabs[i].getAttribute("data-tab") === tabName;
      if (active) {
        tabs[i].setAttribute("class", "detail-tab active");
      } else {
        tabs[i].setAttribute("class", "detail-tab");
      }
    }

    var views = document.getElementsByClassName("detail-view");
    for (var j = 0; j < views.length; j += 1) {
      if (views[j].getAttribute("data-view") === tabName) {
        views[j].setAttribute("class", "detail-view active");
      } else {
        views[j].setAttribute("class", "detail-view");
      }
    }
  }

  function refresh() {
    return call("getDocument").then(function (data) {
      tree = data;
      if (selectedId !== null) {
        if (!findById(tree, selectedId)) {
          selectedId = null;
          clearDetails();
        } else {
          showAttributes(selectedId);
          showStyles(selectedId);
          showComputed(selectedId);
          showLayout(selectedId);
        }
      }
      renderTree();
    });
  }

  function poll() {
    call("getVersion")
      .then(function (data) {
        setStatus("connected");
        var domChanged = data.domVersion !== domVersion;
        var layoutChanged = data.layoutVersion !== layoutVersion;
        if (domChanged) {
          domVersion = data.domVersion;
          layoutVersion = data.layoutVersion;
          return refresh();
        } else if (layoutChanged) {
          layoutVersion = data.layoutVersion;
          if (selectedId !== null) {
            if (!findById(tree, selectedId)) {
              selectedId = null;
              clearDetails();
            } else {
              showComputed(selectedId);
              showLayout(selectedId);
            }
          }
        }
      })
      .catch(function () {
        setStatus("error");
      });
    if (isTraceActive() || isConsoleActive()) {
      call("getTrace")
        .then(function (data) {
          renderTrace(data);
          renderConsoleTrace(data.events || []);
        })
        .catch(function () {});
    }
  }

  function isTraceActive() {
    var tabs = detailTabsEl.getElementsByClassName("detail-tab");
    for (var i = 0; i < tabs.length; i += 1) {
      var tabName = tabs[i].getAttribute("data-tab");
      if ((tabName === "trace" || tabName === "sources") &&
          tabs[i].getAttribute("class").indexOf("active") !== -1) {
        return tabs[i].getAttribute("class").indexOf("active") !== -1;
      }
    }
    return false;
  }

  function isConsoleActive() {
    var tabs = detailTabsEl.getElementsByClassName("detail-tab");
    for (var i = 0; i < tabs.length; i += 1) {
      if (tabs[i].getAttribute("data-tab") === "console" &&
          tabs[i].getAttribute("class").indexOf("active") !== -1) {
        return true;
      }
    }
    return false;
  }

  function renderConsoleTrace(events) {
    for (var i = 0; i < events.length; i += 1) {
      var entry = events[i];
      if (entry.seq <= consoleLastTraceSeq) continue;
      consoleLastTraceSeq = entry.seq;
      if (entry.kind !== "console") continue;
      var level = (entry.label || "console.log").replace("console.", "");
      appendConsoleEntry(level === "error" || level === "warn" ? "error" : "result",
        (entry.label || "console.log") + ": " + (entry.detail || ""));
    }
  }

  function renderActive(active, pendingTasks, timerInFlight) {
    traceActiveEl.textContent = "";
    var items = active || [];
    if (!items.length) {
      var empty = document.createElement("div");
      empty.setAttribute("class", "trace-active-empty");
      if (pendingTasks || timerInFlight) {
        empty.textContent =
          "JS task queued" +
          (pendingTasks ? " (pending: " + pendingTasks + ")" : "") +
          (timerInFlight ? " (timer in flight)" : "") +
          ". No traced operation boundary captured.";
      } else {
        empty.textContent = "No JavaScript currently executing.";
      }
      traceActiveEl.appendChild(empty);
      return;
    }

    for (var i = 0; i < items.length; i += 1) {
      var item = items[i];
      var box = document.createElement("div");
      box.setAttribute("class", "trace-active-item");

      var title = document.createElement("div");
      title.setAttribute("class", "trace-active-title");
      title.textContent =
        "[" + item.kind + "] " + item.label +
        "  " + (item.elapsedMs || 0).toFixed(1) + "ms";
      box.appendChild(title);

      if (item.detail) {
        var detail = document.createElement("div");
        detail.setAttribute("class", "trace-active-meta");
        detail.textContent = item.detail;
        box.appendChild(detail);
      }

      if (item.stack && item.stack.length) {
        var stack = document.createElement("pre");
        stack.setAttribute("class", "trace-active-stack");
        stack.textContent = "Stack:\n" + item.stack.join("\n");
        box.appendChild(stack);
      }

      if (item.source) {
        var source = document.createElement("pre");
        source.setAttribute("class", "trace-active-source");
        source.textContent = item.source;
        box.appendChild(source);
      }

      traceActiveEl.appendChild(box);
    }
  }

  function renderTrace(data) {
    traceEnabled = !!data.enabled;
    traceToggleEl.textContent = traceEnabled ? "Pause" : "Record";
    var status = data.total + " events" +
      (data.dropped ? " · " + data.dropped + " dropped" : "");
    if (data.active && data.active.length) {
      status += " · " + data.active.length + " running";
    } else if (data.pendingTasks) {
      status += " · " + data.pendingTasks + " queued";
    }
    traceStatusEl.textContent = status;
    renderActive(data.active || [], data.pendingTasks || 0, !!data.timerInFlight);
    var events = data.events || [];
    renderSources(events);
    var precedingSource = null;
    for (var sourceIndex = 0; sourceIndex < events.length; sourceIndex += 1) {
      if (events[sourceIndex].source) {
        precedingSource = events[sourceIndex].source;
      } else if (events[sourceIndex].error && precedingSource) {
        events[sourceIndex].source = precedingSource;
        events[sourceIndex].sourceIsFallback = true;
      }
    }
    if (traceLastSeq > data.total) {
      traceEventsEl.textContent = "";
      traceLastSeq = 0;
    }
    var appended = false;
    for (var i = 0; i < events.length; i += 1) {
      (function (entry) {
        if (entry.seq <= traceLastSeq) {
          return;
        }
        var row = document.createElement("button");
        row.setAttribute("class", "trace-row" + (entry.error ? " trace-error" : ""));
        row.textContent = "#" + entry.seq + "  " + entry.atMs.toFixed(1) + "ms  [" +
          entry.kind + "] " + (entry.error ? "ERROR: " + entry.error + "  |  " : "") +
          entry.label + "  " + (entry.detail || "") +
          (entry.error ? " — " + entry.error : "") +
          (entry.durationUs ? "  " + entry.durationUs + "µs" : "");
        row.onclick = function () {
          traceDetailEl.textContent = [
            entry.error ? "Error: " + entry.error : "",
            typeof entry.programCounter === "number"
              ? "Bytecode position: " + entry.programCounter : "",
            entry.stack && entry.stack.length ? "Stack:\n" + entry.stack.join("\n") : "",
            entry.variables && entry.variables.length
              ? "Variables:\n" + entry.variables.map(function (pair) {
                  return pair[0] + " = " + pair[1];
                }).join("\n") : "",
            entry.source ? (entry.sourceIsFallback
              ? "Source (last evaluated script; callback source unavailable):\n"
              : "Source:\n") + entry.source : "",
          ].filter(Boolean).join("\n\n") || entry.detail || entry.label;
        };
        traceEventsEl.appendChild(row);
        if (entry.label === "caught async exception") {
          row.click();
        }
        traceLastSeq = entry.seq;
        appended = true;
      })(events[i]);
    }
    if (appended) {
      traceEventsEl.scrollTop = traceEventsEl.scrollHeight;
    }
  }

  function renderSources(events) {
    for (var i = 0; i < events.length; i += 1) {
      var entry = events[i];
      if (!entry.source) continue;
      var key = entry.label + "\n" + entry.source;
      if (sourceEntries[key]) continue;
      sourceEntries[key] = true;
      var button = document.createElement("button");
      button.setAttribute("class", "source-row");
      button.textContent = entry.label + " — " + entry.detail;
      button.onclick = function (source, label) {
        return function () {
          sourceDetailEl.textContent = "// " + label + "\n" + source;
        };
      }(entry.source, entry.label);
      sourceListEl.appendChild(button);
    }
  }

  traceToggleEl.onclick = function () {
    call("setTracing", { enabled: !traceEnabled }).then(renderTrace);
  };
  document.getElementById("trace-clear").onclick = function () {
    call("clearTrace").then(function (data) {
      traceLastSeq = 0;
      consoleLastTraceSeq = 0;
      traceEventsEl.textContent = "";
      sourceEntries = {};
      sourceListEl.textContent = "";
      sourceDetailEl.textContent = "No JavaScript source recorded yet.";
      traceDetailEl.textContent = "Select an event to inspect its stack and source.";
      renderTrace({ enabled: data.enabled, total: 0, dropped: 0, events: [] });
    });
  };

  function appendConsoleEntry(kind, text) {
    var line = document.createElement("div");
    line.setAttribute("class", "console-entry console-" + kind);
    line.textContent = text;
    consoleOutputEl.appendChild(line);
    consoleOutputEl.scrollTop = consoleOutputEl.scrollHeight;
  }

  function pollConsoleResult(id, source, attempts) {
    attempts = attempts || 0;
    call("getConsoleResult", { id: id }).then(function (data) {
      if (data.done) {
        var value = String(data.value);
        appendConsoleEntry(value.indexOf("Uncaught ") === 0 ? "error" : "result", value);
      } else if (attempts >= 100) {
        appendConsoleEntry("error", "Console evaluation timed out: " + source);
      } else {
        setTimeout(function () { pollConsoleResult(id, source, attempts + 1); }, 50);
      }
    }).catch(function (error) {
      appendConsoleEntry("error", error.message);
    });
  }

  document.getElementById("console-form").onsubmit = function (event) {
    if (event.preventDefault) event.preventDefault();
    var source = consoleInputEl.value;
    if (!source.trim()) return false;
    consoleInputEl.value = "";
    appendConsoleEntry("prompt", "> " + source);
    var id = ++consoleRequestId;
    call("evaluate", { id: id, source: source }).then(function () {
      pollConsoleResult(id, source);
    }).catch(function (error) {
      appendConsoleEntry("error", error.message);
    });
    return false;
  };

  refreshEl.onclick = function () {
    domVersion = null;
    layoutVersion = null;
    poll();
  };

  detailTabsEl.onclick = function (event) {
    var target = event.target;
    if (!target || !target.getAttribute) {
      return;
    }
    var tabName = target.getAttribute("data-tab");
    if (tabName) {
      activateTab(tabName);
      if (tabName === "console") {
        call("getTrace").then(function (data) {
          renderConsoleTrace(data.events || []);
        }).catch(function () {});
      }
    }
  };

  setStatus("connecting");
  poll();
  setInterval(poll, 500);
})();
