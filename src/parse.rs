//! This module contains the parsing code to convert the
//! tokens into an AST.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use either::Either;
use itertools::Itertools;
use miniscript::iter::{Tree, TreeLike};
use pest::Parser;
use pest_derive::Parser;

use crate::error::{Error, RichError, Span, WithFile, WithSpan};
use crate::num::NonZeroPow2Usize;
use crate::pattern::Pattern;
use crate::str::{Binary, Decimal, FunctionName, Hexadecimal, Identifier, JetName, WitnessName};
use crate::types::{AliasedType, BuiltinAlias, TypeConstructible, UIntType};

#[derive(Parser)]
#[grammar = "minimal.pest"]
struct IdentParser;

/// A program is a sequence of items.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Program {
    items: Arc<[Item]>,
    span: Span,
}

impl Program {
    /// Access the items of the program.
    pub fn items(&self) -> &[Item] {
        &self.items
    }
}

/// An item is a component of a program.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Item {
    /// A type alias.
    TypeAlias(TypeAlias),
    /// A function.
    Function(Function),
}

/// Definition of a function.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Function {
    name: FunctionName,
    params: Arc<[FunctionParam]>,
    ret: Option<AliasedType>,
    body: Expression,
    span: Span,
}

impl Function {
    /// Access the name of the function.
    pub fn name(&self) -> &FunctionName {
        &self.name
    }

    /// Access the parameters of the function.
    pub fn params(&self) -> &[FunctionParam] {
        &self.params
    }

    /// Access the return type of the function.
    ///
    /// An empty return type means that the function returns the unit value.
    pub fn ret(&self) -> Option<&AliasedType> {
        self.ret.as_ref()
    }

    /// Access the body of the function.
    pub fn body(&self) -> &Expression {
        &self.body
    }
}

/// Parameter of a function.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct FunctionParam {
    identifier: Identifier,
    ty: AliasedType,
}

impl FunctionParam {
    /// Access the identifier of the parameter.
    pub fn identifier(&self) -> &Identifier {
        &self.identifier
    }

    /// Access the type of the parameter.
    pub fn ty(&self) -> &AliasedType {
        &self.ty
    }
}

/// A statement is a component of a block expression.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Statement {
    /// A declaration of variables inside a pattern.
    Assignment(Assignment),
    /// An expression that returns nothing (the unit value).
    Expression(Expression),
}

/// The output of an expression is assigned to a pattern.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Assignment {
    pattern: Pattern,
    ty: AliasedType,
    expression: Expression,
    span: Span,
}

impl Assignment {
    /// Access the pattern of the assignment.
    pub fn pattern(&self) -> &Pattern {
        &self.pattern
    }

    /// Access the return type of assigned expression.
    pub fn ty(&self) -> &AliasedType {
        &self.ty
    }

    /// Access the assigned expression.
    pub fn expression(&self) -> &Expression {
        &self.expression
    }
}

/// Call expression.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Call {
    name: CallName,
    args: Arc<[Expression]>,
    span: Span,
}

impl Call {
    /// Access the name of the call.
    pub fn name(&self) -> &CallName {
        &self.name
    }

    /// Access the arguments to the call.
    pub fn args(&self) -> &[Expression] {
        self.args.as_ref()
    }
}

/// Name of a call.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum CallName {
    /// Name of a jet.
    Jet(JetName),
    /// [`Either::unwrap_left`].
    UnwrapLeft(AliasedType),
    /// [`Either::unwrap_right`].
    UnwrapRight(AliasedType),
    /// [`Option::unwrap`].
    Unwrap,
    /// [`Option::is_none`].
    IsNone(AliasedType),
    /// [`assert`].
    Assert,
    /// [`panic`] without error message.
    Panic,
    /// Cast from the given source type.
    TypeCast(AliasedType),
    /// Name of a custom function.
    Custom(FunctionName),
    /// Fold of a bounded list with the given function.
    Fold(FunctionName, NonZeroPow2Usize),
    /// Loop over the given function a bounded number of times until it returns success.
    ForWhile(FunctionName),
}

/// A type alias.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TypeAlias {
    name: Identifier,
    ty: AliasedType,
    span: Span,
}

impl TypeAlias {
    /// Access the name of the alias.
    pub fn name(&self) -> &Identifier {
        &self.name
    }

    /// Access the type that the alias resolves to.
    ///
    /// During the parsing stage, the resolved type may include aliases.
    /// The compiler will later check if all contained aliases have been declared before.
    pub fn ty(&self) -> &AliasedType {
        &self.ty
    }
}

/// An expression is something that returns a value.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Expression {
    inner: ExpressionInner,
    span: Span,
}

impl Expression {
    /// Access the inner expression.
    pub fn inner(&self) -> &ExpressionInner {
        &self.inner
    }
}

/// The kind of expression.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ExpressionInner {
    /// A single expression directly returns a value.
    Single(SingleExpression),
    /// A block expression first executes a series of statements inside a local scope.
    /// Then, the block returns the value of its final expression.
    /// The block returns nothing (unit) if there is no final expression.
    Block(Arc<[Statement]>, Option<Arc<Expression>>),
}

/// A single expression directly returns a value.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SingleExpression {
    inner: SingleExpressionInner,
    span: Span,
}

impl SingleExpression {
    /// Access the inner expression.
    pub fn inner(&self) -> &SingleExpressionInner {
        &self.inner
    }
}

/// The kind of single expression.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum SingleExpressionInner {
    /// Either wrapper expression
    Either(Either<Arc<Expression>, Arc<Expression>>),
    /// Option wrapper expression
    Option(Option<Arc<Expression>>),
    /// Boolean literal expression
    Boolean(bool),
    /// Decimal string literal.
    Decimal(Decimal),
    /// Binary string literal.
    Binary(Binary),
    /// Hexadecimal string literal.
    Hexadecimal(Hexadecimal),
    /// Witness identifier expression
    Witness(WitnessName),
    /// Variable identifier expression
    Variable(Identifier),
    /// Function call
    Call(Call),
    /// Expression in parentheses
    Expression(Arc<Expression>),
    /// Match expression over a sum type
    Match(Match),
    /// Tuple wrapper expression
    Tuple(Arc<[Expression]>),
    /// Array wrapper expression
    Array(Arc<[Expression]>),
    /// List wrapper expression
    ///
    /// The exclusive upper bound on the list size is not known at this point
    List(Arc<[Expression]>),
}

/// Match expression.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Match {
    scrutinee: Arc<Expression>,
    left: MatchArm,
    right: MatchArm,
    span: Span,
}

impl Match {
    /// Access the expression that is matched.
    pub fn scrutinee(&self) -> &Expression {
        &self.scrutinee
    }

    /// Access the match arm for left sum values.
    pub fn left(&self) -> &MatchArm {
        &self.left
    }

    /// Access the match arm for right sum values.
    pub fn right(&self) -> &MatchArm {
        &self.right
    }

    /// Get the type of the expression that is matched.
    pub fn scrutinee_type(&self) -> AliasedType {
        match (&self.left.pattern, &self.right.pattern) {
            (MatchPattern::Left(_, ty_l), MatchPattern::Right(_, ty_r)) => {
                AliasedType::either(ty_l.clone(), ty_r.clone())
            }
            (MatchPattern::None, MatchPattern::Some(_, ty_r)) => AliasedType::option(ty_r.clone()),
            (MatchPattern::False, MatchPattern::True) => AliasedType::boolean(),
            _ => unreachable!("Match expressions have valid left and right arms"),
        }
    }
}

/// Arm of a match expression.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct MatchArm {
    pattern: MatchPattern,
    expression: Arc<Expression>,
}

impl MatchArm {
    /// Access the pattern that guards the match arm.
    pub fn pattern(&self) -> &MatchPattern {
        &self.pattern
    }

    /// Access the expression that is executed in the match arm.
    pub fn expression(&self) -> &Expression {
        &self.expression
    }
}

/// Pattern of a match arm.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum MatchPattern {
    /// Bind inner value of left value to variable name.
    Left(Identifier, AliasedType),
    /// Bind inner value of right value to variable name.
    Right(Identifier, AliasedType),
    /// Match none value (no binding).
    None,
    /// Bind inner value of some value to variable name.
    Some(Identifier, AliasedType),
    /// Match false value (no binding).
    False,
    /// Match true value (no binding).
    True,
}

impl MatchPattern {
    /// Access the identifier of a pattern that binds a variable.
    pub fn as_variable(&self) -> Option<&Identifier> {
        match self {
            MatchPattern::Left(i, _) | MatchPattern::Right(i, _) | MatchPattern::Some(i, _) => {
                Some(i)
            }
            MatchPattern::None | MatchPattern::False | MatchPattern::True => None,
        }
    }

    /// Access the identifier and the type of a pattern that binds a variable.
    pub fn as_typed_variable(&self) -> Option<(&Identifier, &AliasedType)> {
        match self {
            MatchPattern::Left(i, ty) | MatchPattern::Right(i, ty) | MatchPattern::Some(i, ty) => {
                Some((i, ty))
            }
            MatchPattern::None | MatchPattern::False | MatchPattern::True => None,
        }
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for item in self.items() {
            writeln!(f, "{item}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TypeAlias(alias) => write!(f, "{alias}"),
            Self::Function(function) => write!(f, "{function}"),
        }
    }
}

impl fmt::Display for TypeAlias {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "type {} = {}", self.name(), self.ty())
    }
}

impl fmt::Display for Function {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fn {}(", self.name())?;
        for (i, param) in self.params().iter().enumerate() {
            if 0 < i {
                write!(f, ", ")?;
            }
            write!(f, "{param}")?;
        }
        write!(f, ")")?;
        if let Some(ty) = self.ret() {
            write!(f, " -> {ty}")?;
        }
        write!(f, " {}", self.body())
    }
}

impl fmt::Display for FunctionParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.identifier(), self.ty())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum ExprTree<'a> {
    Expression(&'a Expression),
    Block(&'a [Statement], &'a Option<Arc<Expression>>),
    Statement(&'a Statement),
    Assignment(&'a Assignment),
    Single(&'a SingleExpression),
    Call(&'a Call),
    Match(&'a Match),
}

impl<'a> TreeLike for ExprTree<'a> {
    fn as_node(&self) -> Tree<Self> {
        use SingleExpressionInner as S;

        match self {
            Self::Expression(expr) => match expr.inner() {
                ExpressionInner::Block(statements, maybe_expr) => {
                    Tree::Unary(ExprTree::Block(statements, maybe_expr))
                }
                ExpressionInner::Single(single) => Tree::Unary(ExprTree::Single(single)),
            },
            Self::Block(statements, maybe_expr) => Tree::Nary(
                statements
                    .iter()
                    .map(ExprTree::Statement)
                    .chain(maybe_expr.iter().map(Arc::as_ref).map(ExprTree::Expression))
                    .collect(),
            ),
            Self::Statement(statement) => match statement {
                Statement::Assignment(assignment) => Tree::Unary(ExprTree::Assignment(assignment)),
                Statement::Expression(expression) => Tree::Unary(ExprTree::Expression(expression)),
            },
            Self::Assignment(assignment) => {
                Tree::Unary(ExprTree::Expression(assignment.expression()))
            }
            Self::Single(single) => match single.inner() {
                S::Boolean(_)
                | S::Binary(_)
                | S::Decimal(_)
                | S::Hexadecimal(_)
                | S::Variable(_)
                | S::Witness(_)
                | S::Option(None) => Tree::Nullary,
                S::Option(Some(l))
                | S::Either(Either::Left(l))
                | S::Either(Either::Right(l))
                | S::Expression(l) => Tree::Unary(Self::Expression(l)),
                S::Call(call) => Tree::Unary(Self::Call(call)),
                S::Match(match_) => Tree::Unary(Self::Match(match_)),
                S::Tuple(tuple) => Tree::Nary(tuple.iter().map(Self::Expression).collect()),
                S::Array(array) => Tree::Nary(array.iter().map(Self::Expression).collect()),
                S::List(list) => Tree::Nary(list.iter().map(Self::Expression).collect()),
            },
            Self::Call(call) => Tree::Nary(call.args().iter().map(Self::Expression).collect()),
            Self::Match(match_) => Tree::Nary(Arc::new([
                Self::Expression(match_.scrutinee()),
                Self::Expression(match_.left().expression()),
                Self::Expression(match_.right().expression()),
            ])),
        }
    }
}

impl<'a> fmt::Display for ExprTree<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use SingleExpressionInner as S;

        for data in self.verbose_pre_order_iter() {
            match &data.node {
                Self::Statement(..) if data.is_complete => writeln!(f, ";")?,
                Self::Expression(..) | Self::Statement(..) => {}
                Self::Block(..) => {
                    if data.n_children_yielded == 0 {
                        writeln!(f, "{{")?;
                    } else if !data.is_complete {
                        write!(f, "    ")?;
                    }
                    if data.is_complete {
                        writeln!(f, "}}")?;
                    }
                }
                Self::Assignment(assignment) => match data.n_children_yielded {
                    0 => write!(f, "let {}: {} = ", assignment.pattern(), assignment.ty())?,
                    n => debug_assert_eq!(n, 1),
                },
                Self::Single(single) => match single.inner() {
                    S::Boolean(bit) => write!(f, "{bit}")?,
                    S::Binary(binary) => write!(f, "0b{binary}")?,
                    S::Decimal(decimal) => write!(f, "{decimal}")?,
                    S::Hexadecimal(hexadecimal) => write!(f, "0x{hexadecimal}")?,
                    S::Variable(name) => write!(f, "{name}")?,
                    S::Witness(name) => write!(f, "witness(\"{name}\")")?,
                    S::Option(None) => write!(f, "None")?,
                    S::Option(Some(_)) => match data.n_children_yielded {
                        0 => write!(f, "Some(")?,
                        n => {
                            debug_assert_eq!(n, 1);
                            write!(f, ")")?;
                        }
                    },
                    S::Either(Either::Left(_)) => match data.n_children_yielded {
                        0 => write!(f, "Left(")?,
                        n => {
                            debug_assert_eq!(n, 1);
                            write!(f, ")")?;
                        }
                    },
                    S::Either(Either::Right(_)) => match data.n_children_yielded {
                        0 => write!(f, "Right(")?,
                        n => {
                            debug_assert_eq!(n, 1);
                            write!(f, ")")?;
                        }
                    },
                    S::Expression(_) => match data.n_children_yielded {
                        0 => write!(f, "(")?,
                        n => {
                            debug_assert_eq!(n, 1);
                            write!(f, ")")?;
                        }
                    },
                    S::Call(..) | S::Match(..) => {}
                    S::Tuple(tuple) => {
                        if data.n_children_yielded == 0 {
                            write!(f, "(")?;
                        } else if !data.is_complete || tuple.len() == 1 {
                            write!(f, ", ")?;
                        }
                        if data.is_complete {
                            write!(f, ")")?;
                        }
                    }
                    S::Array(..) => {
                        if data.n_children_yielded == 0 {
                            write!(f, "[")?;
                        } else if !data.is_complete {
                            write!(f, ", ")?;
                        }
                        if data.is_complete {
                            write!(f, "]")?;
                        }
                    }
                    S::List(..) => {
                        if data.n_children_yielded == 0 {
                            write!(f, "list![")?;
                        } else if !data.is_complete {
                            write!(f, ", ")?;
                        }
                        if data.is_complete {
                            write!(f, "]")?;
                        }
                    }
                },
                Self::Call(call) => {
                    if data.n_children_yielded == 0 {
                        write!(f, "{}(", call.name())?;
                    } else if !data.is_complete {
                        write!(f, ", ")?;
                    }
                    if data.is_complete {
                        write!(f, ")")?;
                    }
                }
                Self::Match(match_) => match data.n_children_yielded {
                    0 => write!(f, "match ")?,
                    1 => write!(f, "{{\n{} => ", match_.left().pattern())?,
                    2 => write!(f, ",\n{} => ", match_.right().pattern())?,
                    n => {
                        debug_assert_eq!(n, 3);
                        write!(f, ",\n}}")?;
                    }
                },
            }
        }

        Ok(())
    }
}

impl fmt::Display for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExprTree::Expression(self))
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExprTree::Statement(self))
    }
}

impl fmt::Display for Assignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExprTree::Assignment(self))
    }
}

impl fmt::Display for SingleExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExprTree::Single(self))
    }
}

impl fmt::Display for Call {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExprTree::Call(self))
    }
}

impl fmt::Display for CallName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallName::Jet(jet) => write!(f, "jet::{jet}"),
            CallName::UnwrapLeft(ty) => write!(f, "unwrap_left::<{ty}>"),
            CallName::UnwrapRight(ty) => write!(f, "unwrap_right::<{ty}>"),
            CallName::Unwrap => write!(f, "unwrap"),
            CallName::IsNone(ty) => write!(f, "is_none::<{ty}>"),
            CallName::Assert => write!(f, "assert!"),
            CallName::Panic => write!(f, "panic!"),
            CallName::TypeCast(ty) => write!(f, "<{ty}>::into"),
            CallName::Custom(name) => write!(f, "{name}"),
            CallName::Fold(name, bound) => write!(f, "fold::<{name}, {bound}>"),
            CallName::ForWhile(name) => write!(f, "for_while::<{name}>"),
        }
    }
}

impl fmt::Display for Match {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", ExprTree::Match(self))
    }
}

impl fmt::Display for MatchPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MatchPattern::Left(i, ty) => write!(f, "Left({i}: {ty})"),
            MatchPattern::Right(i, ty) => write!(f, "Right({i}: {ty})"),
            MatchPattern::None => write!(f, "None"),
            MatchPattern::Some(i, ty) => write!(f, "Some({i}: {ty})"),
            MatchPattern::False => write!(f, "false"),
            MatchPattern::True => write!(f, "true"),
        }
    }
}

/// Trait for types that can be parsed from a PEST pair.
trait PestParse: Sized {
    /// Expected rule for parsing the type.
    const RULE: Rule;

    /// Parse a value of the type from a PEST pair.
    ///
    /// # Panics
    ///
    /// The rule of the pair is not the expected rule ([`Self::RULE`]).
    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError>;
}

/// Copy of [`FromStr`] that internally uses the PEST parser.
pub trait ParseFromStr: Sized {
    /// Parse a value from the string `s`.
    fn parse_from_str(s: &str) -> Result<Self, RichError>;
}

impl<A: PestParse> ParseFromStr for A {
    fn parse_from_str(s: &str) -> Result<Self, RichError> {
        let mut pairs = IdentParser::parse(A::RULE, s)
            .map_err(RichError::from)
            .with_file(s)?;
        let pair = pairs.next().unwrap();
        A::parse(pair).with_file(s)
    }
}

impl PestParse for Program {
    const RULE: Rule = Rule::program;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let span = Span::from(&pair);
        let items = pair
            .into_inner()
            .filter_map(|pair| match pair.as_rule() {
                Rule::item => Some(Item::parse(pair)),
                Rule::EOI => None,
                _ => unreachable!("Corrupt grammar"),
            })
            .collect::<Result<Arc<[Item]>, RichError>>()?;
        Ok(Program { items, span })
    }
}

impl PestParse for Item {
    const RULE: Rule = Rule::item;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let pair = pair.into_inner().next().unwrap();
        match pair.as_rule() {
            Rule::type_alias => TypeAlias::parse(pair).map(Item::TypeAlias),
            Rule::function => Function::parse(pair).map(Item::Function),
            _ => unreachable!("Corrupt grammar"),
        }
    }
}

impl PestParse for Function {
    const RULE: Rule = Rule::function;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let span = Span::from(&pair);
        let mut it = pair.into_inner();
        let _fn_keyword = it.next().unwrap();
        let name = FunctionName::parse(it.next().unwrap())?;
        let params = {
            let pair = it.next().unwrap();
            debug_assert!(matches!(pair.as_rule(), Rule::function_params));
            pair.into_inner()
                .map(FunctionParam::parse)
                .collect::<Result<Arc<[FunctionParam]>, RichError>>()?
        };
        let ret = match it.peek().unwrap().as_rule() {
            Rule::function_return => {
                let pair = it.next().unwrap();
                debug_assert!(matches!(pair.as_rule(), Rule::function_return));
                let pair = pair.into_inner().next().unwrap();
                let ty = AliasedType::parse(pair)?;
                Some(ty)
            }
            _ => None,
        };
        let body = Expression::parse(it.next().unwrap())?;

        Ok(Self {
            name,
            params,
            ret,
            body,
            span,
        })
    }
}

impl PestParse for FunctionName {
    const RULE: Rule = Rule::function_name;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        Ok(Self::from_str_unchecked(pair.as_str()))
    }
}

impl PestParse for FunctionParam {
    const RULE: Rule = Rule::typed_identifier;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let mut it = pair.into_inner();
        let identifier = Identifier::parse(it.next().unwrap())?;
        let ty = AliasedType::parse(it.next().unwrap())?;
        Ok(Self { identifier, ty })
    }
}

impl PestParse for Statement {
    const RULE: Rule = Rule::statement;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let inner_pair = pair.into_inner().next().unwrap();
        match inner_pair.as_rule() {
            Rule::assignment => Assignment::parse(inner_pair).map(Statement::Assignment),
            Rule::expression => Expression::parse(inner_pair).map(Statement::Expression),
            _ => unreachable!("Corrupt grammar"),
        }
    }
}

impl PestParse for Pattern {
    const RULE: Rule = Rule::pattern;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let pair = PatternPair(pair);
        let mut output = vec![];

        for data in pair.post_order_iter() {
            match data.node.0.as_rule() {
                Rule::pattern => {}
                Rule::variable_pattern => {
                    let identifier = Identifier::parse(data.node.0.into_inner().next().unwrap())?;
                    output.push(Pattern::Identifier(identifier));
                }
                Rule::ignore_pattern => {
                    output.push(Pattern::Ignore);
                }
                Rule::tuple_pattern => {
                    let size = data.node.n_children();
                    let elements = output.split_off(output.len() - size);
                    debug_assert_eq!(elements.len(), size);
                    output.push(Pattern::tuple(elements));
                }
                Rule::array_pattern => {
                    let size = data.node.n_children();
                    let elements = output.split_off(output.len() - size);
                    debug_assert_eq!(elements.len(), size);
                    output.push(Pattern::array(elements));
                }
                _ => unreachable!("Corrupt grammar"),
            }
        }

        debug_assert!(output.len() == 1);
        Ok(output.pop().unwrap())
    }
}

impl PestParse for Identifier {
    const RULE: Rule = Rule::identifier;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        Ok(Self::from_str_unchecked(pair.as_str()))
    }
}

impl PestParse for Assignment {
    const RULE: Rule = Rule::assignment;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let span = Span::from(&pair);
        let mut it = pair.into_inner();
        let _let_keyword = it.next().unwrap();
        let pattern = Pattern::parse(it.next().unwrap())?;
        let ty = AliasedType::parse(it.next().unwrap())?;
        let expression = Expression::parse(it.next().unwrap())?;
        Ok(Assignment {
            pattern,
            ty,
            expression,
            span,
        })
    }
}

impl PestParse for Call {
    const RULE: Rule = Rule::call_expr;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let span = Span::from(&pair);
        let mut it = pair.into_inner();
        let name = CallName::parse(it.next().unwrap())?;
        let args = {
            let pair = it.next().unwrap();
            debug_assert!(matches!(pair.as_rule(), Rule::call_args));
            pair.into_inner()
                .map(Expression::parse)
                .collect::<Result<Arc<[Expression]>, RichError>>()?
        };

        Ok(Self { name, args, span })
    }
}

impl PestParse for CallName {
    const RULE: Rule = Rule::call_name;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let pair = pair.into_inner().next().unwrap();
        match pair.as_rule() {
            Rule::jet => JetName::parse(pair).map(Self::Jet),
            Rule::unwrap_left => {
                let inner = pair.into_inner().next().unwrap();
                AliasedType::parse(inner).map(Self::UnwrapLeft)
            }
            Rule::unwrap_right => {
                let inner = pair.into_inner().next().unwrap();
                AliasedType::parse(inner).map(Self::UnwrapRight)
            }
            Rule::is_none => {
                let inner = pair.into_inner().next().unwrap();
                AliasedType::parse(inner).map(Self::IsNone)
            }
            Rule::unwrap => Ok(Self::Unwrap),
            Rule::assert => Ok(Self::Assert),
            Rule::panic => Ok(Self::Panic),
            Rule::type_cast => {
                let inner = pair.into_inner().next().unwrap();
                AliasedType::parse(inner).map(Self::TypeCast)
            }
            Rule::fold => {
                let mut it = pair.into_inner();
                let name = FunctionName::parse(it.next().unwrap())?;
                let bound = NonZeroPow2Usize::parse(it.next().unwrap())?;
                Ok(Self::Fold(name, bound))
            }
            Rule::for_while => {
                let mut it = pair.into_inner();
                let name = FunctionName::parse(it.next().unwrap())?;
                Ok(Self::ForWhile(name))
            }
            Rule::function_name => FunctionName::parse(pair).map(Self::Custom),
            _ => panic!("Corrupt grammar"),
        }
    }
}

impl PestParse for JetName {
    const RULE: Rule = Rule::jet;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let jet_name = pair.as_str().strip_prefix("jet::").unwrap();
        Ok(Self::from_str_unchecked(jet_name))
    }
}

impl PestParse for TypeAlias {
    const RULE: Rule = Rule::type_alias;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let span = Span::from(&pair);
        let mut it = pair.into_inner();
        let _type_keyword = it.next().unwrap();
        let name = Identifier::parse(it.next().unwrap().into_inner().next().unwrap())?;
        let ty = AliasedType::parse(it.next().unwrap())?;
        Ok(Self { name, ty, span })
    }
}

impl PestParse for Expression {
    const RULE: Rule = Rule::expression;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        let span = Span::from(&pair);
        let pair = match pair.as_rule() {
            Rule::expression => pair.into_inner().next().unwrap(),
            Rule::block_expression | Rule::single_expression => pair,
            _ => unreachable!("Corrupt grammar"),
        };

        let inner = match pair.as_rule() {
            Rule::block_expression => {
                let mut it = pair.into_inner().peekable();
                let statements = it
                    .peeking_take_while(|pair| matches!(pair.as_rule(), Rule::statement))
                    .map(Statement::parse)
                    .collect::<Result<Arc<[Statement]>, RichError>>()?;
                let expression = it
                    .next()
                    .map(|pair| Expression::parse(pair).map(Arc::new))
                    .transpose()?;
                ExpressionInner::Block(statements, expression)
            }
            Rule::single_expression => ExpressionInner::Single(SingleExpression::parse(pair)?),
            _ => unreachable!("Corrupt grammar"),
        };

        Ok(Expression { inner, span })
    }
}

impl PestParse for SingleExpression {
    const RULE: Rule = Rule::single_expression;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));

        let span = Span::from(&pair);
        let inner_pair = pair.into_inner().next().unwrap();

        let inner = match inner_pair.as_rule() {
            Rule::left_expr => {
                let l = inner_pair.into_inner().next().unwrap();
                Expression::parse(l)
                    .map(Arc::new)
                    .map(Either::Left)
                    .map(SingleExpressionInner::Either)?
            }
            Rule::right_expr => {
                let r = inner_pair.into_inner().next().unwrap();
                Expression::parse(r)
                    .map(Arc::new)
                    .map(Either::Right)
                    .map(SingleExpressionInner::Either)?
            }
            Rule::none_expr => SingleExpressionInner::Option(None),
            Rule::some_expr => {
                let r = inner_pair.into_inner().next().unwrap();
                Expression::parse(r)
                    .map(Arc::new)
                    .map(Some)
                    .map(SingleExpressionInner::Option)?
            }
            Rule::false_expr => SingleExpressionInner::Boolean(false),
            Rule::true_expr => SingleExpressionInner::Boolean(true),
            Rule::call_expr => SingleExpressionInner::Call(Call::parse(inner_pair)?),
            Rule::bin_literal => Binary::parse(inner_pair).map(SingleExpressionInner::Binary)?,
            Rule::hex_literal => {
                Hexadecimal::parse(inner_pair).map(SingleExpressionInner::Hexadecimal)?
            }
            Rule::dec_literal => Decimal::parse(inner_pair).map(SingleExpressionInner::Decimal)?,
            Rule::witness_expr => {
                let witness_pair = inner_pair.into_inner().next().unwrap();
                SingleExpressionInner::Witness(WitnessName::parse(witness_pair)?)
            }
            Rule::variable_expr => {
                let identifier_pair = inner_pair.into_inner().next().unwrap();
                SingleExpressionInner::Variable(Identifier::parse(identifier_pair)?)
            }
            Rule::expression => {
                SingleExpressionInner::Expression(Expression::parse(inner_pair).map(Arc::new)?)
            }
            Rule::match_expr => Match::parse(inner_pair).map(SingleExpressionInner::Match)?,
            Rule::tuple_expr => inner_pair
                .clone()
                .into_inner()
                .map(Expression::parse)
                .collect::<Result<Arc<[Expression]>, _>>()
                .map(SingleExpressionInner::Tuple)?,
            Rule::array_expr => inner_pair
                .clone()
                .into_inner()
                .map(Expression::parse)
                .collect::<Result<Arc<[Expression]>, _>>()
                .map(SingleExpressionInner::Array)?,
            Rule::list_expr => {
                let elements = inner_pair
                    .into_inner()
                    .map(|inner| Expression::parse(inner))
                    .collect::<Result<Arc<_>, _>>()?;
                SingleExpressionInner::List(elements)
            }
            _ => unreachable!("Corrupt grammar"),
        };

        Ok(SingleExpression { inner, span })
    }
}

impl PestParse for Decimal {
    const RULE: Rule = Rule::dec_literal;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let decimal = pair.as_str().replace('_', "");
        Ok(Self::from_str_unchecked(decimal.as_str()))
    }
}

impl PestParse for Binary {
    const RULE: Rule = Rule::bin_literal;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let binary = pair.as_str().strip_prefix("0b").unwrap().replace('_', "");
        Ok(Self::from_str_unchecked(binary.as_str()))
    }
}

impl PestParse for Hexadecimal {
    const RULE: Rule = Rule::hex_literal;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let hexadecimal = pair.as_str().strip_prefix("0x").unwrap().replace('_', "");
        Ok(Self::from_str_unchecked(hexadecimal.as_str()))
    }
}

impl PestParse for WitnessName {
    const RULE: Rule = Rule::witness_name;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        Ok(Self::from_str_unchecked(pair.as_str()))
    }
}

impl PestParse for Match {
    const RULE: Rule = Rule::match_expr;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let span = Span::from(&pair);
        let mut it = pair.into_inner();
        let _match_keyword = it.next().unwrap();
        let scrutinee_pair = it.next().unwrap();
        let scrutinee = Expression::parse(scrutinee_pair.clone()).map(Arc::new)?;
        let first = MatchArm::parse(it.next().unwrap())?;
        let second = MatchArm::parse(it.next().unwrap())?;

        let (left, right) = match (&first.pattern, &second.pattern) {
            (MatchPattern::Left(..), MatchPattern::Right(..)) => (first, second),
            (MatchPattern::Right(..), MatchPattern::Left(..)) => (second, first),
            (MatchPattern::None, MatchPattern::Some(..)) => (first, second),
            (MatchPattern::False, MatchPattern::True) => (first, second),
            (MatchPattern::Some(..), MatchPattern::None) => (second, first),
            (MatchPattern::True, MatchPattern::False) => (second, first),
            (p1, p2) => {
                return Err(Error::IncompatibleMatchArms(p1.clone(), p2.clone())).with_span(span)
            }
        };

        Ok(Self {
            scrutinee,
            left,
            right,
            span,
        })
    }
}

impl PestParse for MatchArm {
    const RULE: Rule = Rule::match_arm;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let mut it = pair.into_inner();
        let pattern = MatchPattern::parse(it.next().unwrap())?;
        let expression = Expression::parse(it.next().unwrap()).map(Arc::new)?;
        Ok(MatchArm {
            pattern,
            expression,
        })
    }
}

impl PestParse for MatchPattern {
    const RULE: Rule = Rule::match_pattern;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let pair = pair.into_inner().next().unwrap();
        let ret = match pair.as_rule() {
            rule @ (Rule::left_pattern | Rule::right_pattern | Rule::some_pattern) => {
                let mut it = pair.into_inner();
                let identifier = Identifier::parse(it.next().unwrap())?;
                let ty = AliasedType::parse(it.next().unwrap())?;

                match rule {
                    Rule::left_pattern => MatchPattern::Left(identifier, ty),
                    Rule::right_pattern => MatchPattern::Right(identifier, ty),
                    Rule::some_pattern => MatchPattern::Some(identifier, ty),
                    _ => unreachable!("Covered by outer match"),
                }
            }
            Rule::none_pattern => MatchPattern::None,
            Rule::false_pattern => MatchPattern::False,
            Rule::true_pattern => MatchPattern::True,
            _ => unreachable!("Corrupt grammar"),
        };
        Ok(ret)
    }
}

impl PestParse for AliasedType {
    const RULE: Rule = Rule::ty;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        enum Item {
            Type(AliasedType),
            Size(usize),
            Bound(NonZeroPow2Usize),
        }

        impl Item {
            fn unwrap_type(self) -> AliasedType {
                match self {
                    Item::Type(ty) => ty,
                    _ => panic!("Not a type"),
                }
            }

            fn unwrap_size(self) -> usize {
                match self {
                    Item::Size(size) => size,
                    _ => panic!("Not a size"),
                }
            }

            fn unwrap_bound(self) -> NonZeroPow2Usize {
                match self {
                    Item::Bound(size) => size,
                    _ => panic!("Not a bound"),
                }
            }
        }

        assert!(matches!(pair.as_rule(), Self::RULE));
        let pair = TyPair(pair);
        let mut output = vec![];

        for data in pair.post_order_iter() {
            match data.node.0.as_rule() {
                Rule::alias_name => {
                    let pair = data.node.0.into_inner().next().unwrap();
                    let identifier = Identifier::parse(pair)?;
                    output.push(Item::Type(AliasedType::alias(identifier)));
                }
                Rule::builtin_alias => {
                    let builtin = BuiltinAlias::parse(data.node.0)?;
                    output.push(Item::Type(AliasedType::builtin(builtin)));
                }
                Rule::unsigned_type => {
                    let uint_ty = UIntType::parse(data.node.0)?;
                    output.push(Item::Type(AliasedType::from(uint_ty)));
                }
                Rule::sum_type => {
                    let r = output.pop().unwrap().unwrap_type();
                    let l = output.pop().unwrap().unwrap_type();
                    output.push(Item::Type(AliasedType::either(l, r)));
                }
                Rule::option_type => {
                    let r = output.pop().unwrap().unwrap_type();
                    output.push(Item::Type(AliasedType::option(r)));
                }
                Rule::boolean_type => {
                    output.push(Item::Type(AliasedType::boolean()));
                }
                Rule::tuple_type => {
                    let size = data.node.n_children();
                    let elements: Vec<AliasedType> = output
                        .split_off(output.len() - size)
                        .into_iter()
                        .map(Item::unwrap_type)
                        .collect();
                    debug_assert_eq!(elements.len(), size);
                    output.push(Item::Type(AliasedType::tuple(elements)));
                }
                Rule::array_type => {
                    let size = output.pop().unwrap().unwrap_size();
                    let el = output.pop().unwrap().unwrap_type();
                    output.push(Item::Type(AliasedType::array(el, size)));
                }
                Rule::array_size => {
                    let size_str = data.node.0.as_str();
                    let size = size_str.parse::<usize>().with_span(&data.node.0)?;
                    output.push(Item::Size(size));
                }
                Rule::list_type => {
                    let bound = output.pop().unwrap().unwrap_bound();
                    let el = output.pop().unwrap().unwrap_type();
                    output.push(Item::Type(AliasedType::list(el, bound)));
                }
                Rule::list_bound => {
                    let bound = NonZeroPow2Usize::parse(data.node.0)?;
                    output.push(Item::Bound(bound));
                }
                Rule::ty => {}
                _ => unreachable!("Corrupt grammar"),
            }
        }

        debug_assert!(output.len() == 1);
        Ok(output.pop().unwrap().unwrap_type())
    }
}

impl PestParse for UIntType {
    const RULE: Rule = Rule::unsigned_type;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let ret = match pair.as_str() {
            "u1" => UIntType::U1,
            "u2" => UIntType::U2,
            "u4" => UIntType::U4,
            "u8" => UIntType::U8,
            "u16" => UIntType::U16,
            "u32" => UIntType::U32,
            "u64" => UIntType::U64,
            "u128" => UIntType::U128,
            "u256" => UIntType::U256,
            _ => unreachable!("Corrupt grammar"),
        };
        Ok(ret)
    }
}

impl PestParse for BuiltinAlias {
    const RULE: Rule = Rule::builtin_alias;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        Self::from_str(pair.as_str())
            .map_err(Error::CannotParse)
            .with_span(&pair)
    }
}

impl PestParse for NonZeroPow2Usize {
    // FIXME: This equates NonZeroPow2Usize with list bounds. Create wrapper for list bounds?
    const RULE: Rule = Rule::list_bound;

    fn parse(pair: pest::iterators::Pair<Rule>) -> Result<Self, RichError> {
        assert!(matches!(pair.as_rule(), Self::RULE));
        let bound = pair.as_str().parse::<usize>().with_span(&pair)?;
        NonZeroPow2Usize::new(bound)
            .ok_or(Error::ListBoundPow2(bound))
            .with_span(&pair)
    }
}

/// Pair of tokens from the 'pattern' rule.
#[derive(Clone, Debug)]
struct PatternPair<'a>(pest::iterators::Pair<'a, Rule>);

impl<'a> TreeLike for PatternPair<'a> {
    fn as_node(&self) -> Tree<Self> {
        let mut it = self.0.clone().into_inner();
        match self.0.as_rule() {
            Rule::variable_pattern | Rule::ignore_pattern => Tree::Nullary,
            Rule::pattern => {
                let l = it.next().unwrap();
                Tree::Unary(PatternPair(l))
            }
            Rule::tuple_pattern | Rule::array_pattern => {
                let children: Arc<[PatternPair]> = it.map(PatternPair).collect();
                Tree::Nary(children)
            }
            _ => unreachable!("Corrupt grammar"),
        }
    }
}

/// Pair of tokens from the 'ty' rule.
#[derive(Clone, Debug)]
struct TyPair<'a>(pest::iterators::Pair<'a, Rule>);

impl<'a> TreeLike for TyPair<'a> {
    fn as_node(&self) -> Tree<Self> {
        let mut it = self.0.clone().into_inner();
        match self.0.as_rule() {
            Rule::boolean_type
            | Rule::unsigned_type
            | Rule::array_size
            | Rule::list_bound
            | Rule::alias_name
            | Rule::builtin_alias => Tree::Nullary,
            Rule::ty | Rule::option_type => {
                let l = it.next().unwrap();
                Tree::Unary(TyPair(l))
            }
            Rule::sum_type | Rule::array_type | Rule::list_type => {
                let l = it.next().unwrap();
                let r = it.next().unwrap();
                Tree::Binary(TyPair(l), TyPair(r))
            }
            Rule::tuple_type => Tree::Nary(it.map(TyPair).collect()),
            _ => unreachable!("Corrupt grammar"),
        }
    }
}

impl<'a, A: AsRef<Span>> From<&'a A> for Span {
    fn from(value: &'a A) -> Self {
        *value.as_ref()
    }
}

impl AsRef<Span> for Program {
    fn as_ref(&self) -> &Span {
        &self.span
    }
}

impl AsRef<Span> for Function {
    fn as_ref(&self) -> &Span {
        &self.span
    }
}

impl AsRef<Span> for Assignment {
    fn as_ref(&self) -> &Span {
        &self.span
    }
}

impl AsRef<Span> for TypeAlias {
    fn as_ref(&self) -> &Span {
        &self.span
    }
}

impl AsRef<Span> for Expression {
    fn as_ref(&self) -> &Span {
        &self.span
    }
}

impl AsRef<Span> for SingleExpression {
    fn as_ref(&self) -> &Span {
        &self.span
    }
}

impl AsRef<Span> for Call {
    fn as_ref(&self) -> &Span {
        &self.span
    }
}

impl AsRef<Span> for Match {
    fn as_ref(&self) -> &Span {
        &self.span
    }
}
