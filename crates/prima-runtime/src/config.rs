use prima_core::number::Number;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Real,
    Complex,
    Integer,
    Positive,
    NonNegative,
    NonZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndefinedHandling {
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintFormat {
    Latex,
    Unicode,
    Ascii,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub domain: Domain,
    pub undefined_handling: UndefinedHandling,
    pub fraction: bool,
    pub broadcast: bool,
    pub loop_optimization: bool,
    pub simplify_level: u8,
    pub num_to_big: bool,
    pub print_format: PrintFormat,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            domain: Domain::Complex,
            undefined_handling: UndefinedHandling::Strict,
            fraction: true,
            broadcast: true,
            loop_optimization: true,
            simplify_level: 2,
            num_to_big: true,
            print_format: PrintFormat::Latex,
        }
    }
}

pub struct Engine {
    pub config: Config,
    pub _marker: std::marker::PhantomData<Number>,
}
