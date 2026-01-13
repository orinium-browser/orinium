use crate::engine::css::values::length::{Length as CssLength, LengthKeyword, LengthUnit};
use ui_layout::Length;

#[derive(Debug, Clone, PartialEq)]
pub enum LengthConvertError {
    UnsupportedUnit(LengthUnit),
    UnsupportedKeyword(LengthKeyword),
}

impl TryFrom<CssLength> for Length {
    type Error = LengthConvertError;

    fn try_from(value: CssLength) -> Result<Self, Self::Error> {
        match value {
            CssLength::Value(v, unit) => match unit {
                LengthUnit::Px => Ok(Length::Px(v)),
                LengthUnit::Percent => Ok(Length::Percent(v)),
                LengthUnit::Vw => Ok(Length::Vw(v)),
                LengthUnit::Vh => Ok(Length::Vh(v)),
                _ => Err(LengthConvertError::UnsupportedUnit(unit)),
            },
            CssLength::Keyword(k) => match k {
                LengthKeyword::Auto => Ok(Length::Auto),
                _ => Err(LengthConvertError::UnsupportedKeyword(k)),
            },
        }
    }
}

impl TryFrom<&CssLength> for Length {
    type Error = LengthConvertError;

    fn try_from(value: &CssLength) -> Result<Self, Self::Error> {
        match value {
            CssLength::Value(v, unit) => match unit {
                LengthUnit::Px => Ok(Length::Px(*v)),
                LengthUnit::Percent => Ok(Length::Percent(*v)),
                LengthUnit::Vw => Ok(Length::Vw(*v)),
                LengthUnit::Vh => Ok(Length::Vh(*v)),
                _ => Err(LengthConvertError::UnsupportedUnit(*unit)),
            },
            CssLength::Keyword(k) => match k {
                LengthKeyword::Auto => Ok(Length::Auto),
                _ => Err(LengthConvertError::UnsupportedKeyword(*k)),
            },
        }
    }
}
