//! Cold recipe construction for detached command continuations.

use super::schema::{
    CommandSummaryRecipe, ContinuationSchema, DetachedAttemptRecipe, DetachedCommandProfile,
    GlueRecipe, GlueRecipeIndex, MacroRecipe, MacroRecipeIndex, NameRecipe, NameRecipeIndex,
    OriginListRecipe, OriginListRecipeIndex, OriginRecipe, OriginRecipeIndex, SourceRecipe,
    SourceRecipeIndex, TokenListRecipe, TokenListRecipeIndex,
};
use super::{CommandContinuationError, OwnedCommandContinuation};

/// Builder used by the cold detacher while walking an explicit root set.
///
/// Each insertion returns the dense DTO-local index of the appended recipe.
/// The builder has no access to destination storage and retains no runtime
/// coordinate after the caller has translated a value into its logical form.
pub(crate) struct ContinuationRecipeBuilder {
    profile: DetachedCommandProfile,
    sources: Vec<SourceRecipe>,
    names: Vec<NameRecipe>,
    token_lists: Vec<TokenListRecipe>,
    origins: Vec<OriginRecipe>,
    origin_lists: Vec<OriginListRecipe>,
    macros: Vec<MacroRecipe>,
    glue: Vec<GlueRecipe>,
}

impl ContinuationRecipeBuilder {
    #[must_use]
    pub(crate) const fn new(profile: DetachedCommandProfile) -> Self {
        Self {
            profile,
            sources: Vec::new(),
            names: Vec::new(),
            token_lists: Vec::new(),
            origins: Vec::new(),
            origin_lists: Vec::new(),
            macros: Vec::new(),
            glue: Vec::new(),
        }
    }

    pub(crate) fn push_source(
        &mut self,
        recipe: SourceRecipe,
    ) -> Result<SourceRecipeIndex, CommandContinuationError> {
        let index = SourceRecipeIndex::from_len(self.sources.len()).ok_or(
            CommandContinuationError::LimitExceeded("source recipe index"),
        )?;
        self.sources.push(recipe);
        Ok(index)
    }

    pub(crate) fn push_name(
        &mut self,
        recipe: NameRecipe,
    ) -> Result<NameRecipeIndex, CommandContinuationError> {
        let index = NameRecipeIndex::from_len(self.names.len())
            .ok_or(CommandContinuationError::LimitExceeded("name recipe index"))?;
        self.names.push(recipe);
        Ok(index)
    }

    pub(crate) fn push_token_list(
        &mut self,
        recipe: TokenListRecipe,
    ) -> Result<TokenListRecipeIndex, CommandContinuationError> {
        let index = TokenListRecipeIndex::from_len(self.token_lists.len()).ok_or(
            CommandContinuationError::LimitExceeded("token-list recipe index"),
        )?;
        self.token_lists.push(recipe);
        Ok(index)
    }

    pub(crate) fn push_origin(
        &mut self,
        recipe: OriginRecipe,
    ) -> Result<OriginRecipeIndex, CommandContinuationError> {
        let index = OriginRecipeIndex::from_len(self.origins.len()).ok_or(
            CommandContinuationError::LimitExceeded("origin recipe index"),
        )?;
        self.origins.push(recipe);
        Ok(index)
    }

    pub(crate) fn push_origin_list(
        &mut self,
        recipe: OriginListRecipe,
    ) -> Result<OriginListRecipeIndex, CommandContinuationError> {
        let index = OriginListRecipeIndex::from_len(self.origin_lists.len()).ok_or(
            CommandContinuationError::LimitExceeded("origin-list recipe index"),
        )?;
        self.origin_lists.push(recipe);
        Ok(index)
    }

    pub(crate) fn push_macro(
        &mut self,
        recipe: MacroRecipe,
    ) -> Result<MacroRecipeIndex, CommandContinuationError> {
        let index = MacroRecipeIndex::from_len(self.macros.len()).ok_or(
            CommandContinuationError::LimitExceeded("macro recipe index"),
        )?;
        self.macros.push(recipe);
        Ok(index)
    }

    pub(crate) fn push_glue(
        &mut self,
        recipe: GlueRecipe,
    ) -> Result<GlueRecipeIndex, CommandContinuationError> {
        let index = GlueRecipeIndex::from_len(self.glue.len())
            .ok_or(CommandContinuationError::LimitExceeded("glue recipe index"))?;
        self.glue.push(recipe);
        Ok(index)
    }

    pub(crate) fn finish(
        self,
        summary: CommandSummaryRecipe,
        attempt: Option<DetachedAttemptRecipe>,
    ) -> Result<OwnedCommandContinuation, CommandContinuationError> {
        OwnedCommandContinuation::from_schema(ContinuationSchema {
            profile: self.profile,
            summary,
            attempt,
            sources: self.sources,
            names: self.names,
            token_lists: self.token_lists,
            origins: self.origins,
            origin_lists: self.origin_lists,
            macros: self.macros,
            glue: self.glue,
        })
    }
}
