/*
 *  This file is licensed under the Apache 2.0 License.
 *  (c) 2025 orinium-browser
 *  created by nekogakure
 */

/// Search handler
document.addEventListener('DOMContentLoaded', () => {
    const form = document.querySelector('.search');
    const input = document.getElementById('q');

    if (!form || !input) return;

    const originalPlaceholder = input.getAttribute('placeholder') || '';

    const removePlaceholder = () => {
        if (input.hasAttribute('placeholder')) {
            input.removeAttribute('placeholder');
        }
    };

    const restorePlaceholderIfEmpty = () => {
        if (!input.value.trim()) {
            input.setAttribute('placeholder', originalPlaceholder);
        }
    };

    input.addEventListener('click', removePlaceholder);
    input.addEventListener('focus', removePlaceholder);
    input.addEventListener('mousedown', (e) => {
        removePlaceholder();
    });

    input.addEventListener('input', () => {
        if (input.value.trim()) {
            removePlaceholder();
        }
    });

    input.addEventListener('blur', restorePlaceholderIfEmpty);

    form.addEventListener('submit', (e) => {
        e.preventDefault();
        const query = input.value.trim();
        if (!query) return;
        window.location.href = 'https://www.google.com/search?q=' + encodeURIComponent(query);
    });
});