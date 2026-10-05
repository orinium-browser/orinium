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
  var version = "";

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
        var next = data.domVersion + "/" + data.layoutVersion;
        if (next !== version) {
          version = next;
          return refresh();
        }
      })
      .catch(function () {
        setStatus("error");
      });
  }

  refreshEl.onclick = function () {
    version = "";
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
    }
  };

  setStatus("connecting");
  poll();
  setInterval(poll, 500);
})();
