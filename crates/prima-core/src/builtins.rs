use std::sync::OnceLock;

use crate::symbol::{SymbolId, SymbolTable};

pub struct BuiltinSymbols {
    pub e: SymbolId,
    pub pi: SymbolId,
    pub i: SymbolId,
    pub tau: SymbolId,
    pub inf: SymbolId,
    pub gamma: SymbolId,
    pub phi: SymbolId,
    pub sqrt: SymbolId,
    pub exp: SymbolId,
    pub log: SymbolId,
    pub ln: SymbolId,
    pub sin: SymbolId,
    pub cos: SymbolId,
    pub tan: SymbolId,
    pub sigma: SymbolId,
    pub prod: SymbolId,
    pub int: SymbolId,
    pub partial: SymbolId,
    pub abs: SymbolId,
}

pub(crate) fn register(t: &SymbolTable) {
    t.intern_display("e", "\\e");
    t.intern_display("pi", "\\pi");
    t.intern_display("i", "\\i");
    t.intern_display("tau", "\\tau");
    t.intern_display("infty", "\\infty");
    t.intern_display("gamma", "\\gamma");
    t.intern_display("phi", "\\phi");
    t.intern_display("sqrt", "\\sqrt");
    t.intern_display("exp", "\\exp");
    t.intern_display("log", "\\log");
    t.intern_display("ln", "\\ln");
    t.intern_display("sin", "\\sin");
    t.intern_display("cos", "\\cos");
    t.intern_display("tan", "\\tan");
    t.intern_display("sigma", "\\sigma");
    t.intern_display("prod", "\\prod");
    t.intern_display("int", "\\int");
    t.intern_display("partial", "\\partial");
    t.intern_display("abs", "\\mathrm{abs}");
}

impl BuiltinSymbols {
    pub fn global() -> &'static BuiltinSymbols {
        static B: OnceLock<BuiltinSymbols> = OnceLock::new();
        B.get_or_init(|| {
            let t = SymbolTable::global();
            BuiltinSymbols {
                e: t.intern("e"),
                pi: t.intern("pi"),
                i: t.intern("i"),
                tau: t.intern("tau"),
                inf: t.intern("infty"),
                gamma: t.intern("gamma"),
                phi: t.intern("phi"),
                sqrt: t.intern("sqrt"),
                exp: t.intern("exp"),
                log: t.intern("log"),
                ln: t.intern("ln"),
                sin: t.intern("sin"),
                cos: t.intern("cos"),
                tan: t.intern("tan"),
                sigma: t.intern("sigma"),
                prod: t.intern("prod"),
                int: t.intern("int"),
                partial: t.intern("partial"),
                abs: t.intern("abs"),
            }
        })
    }
}
