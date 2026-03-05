# frozen_string_literal: true

module MudAdapter
  module Stdlib
    module World
      class ReviewPolicy
        def initialize
          @protect_main = false
          @required_approvals = 0
          @reviewer_check = ->(_user, _area) { true }
          @dev_access_check = ->(_user, _area) { true }
          @mr_create_check = ->(_user, _area) { true }
        end

        def configure(&)
          dsl = PolicyDSL.new(self)
          dsl.instance_eval(&)
        end

        def main_protected?
          @protect_main
        end

        def can_approve?(user, area)
          @reviewer_check.call(user, area)
        end

        def can_create_merge_request?(user, area)
          @mr_create_check.call(user, area)
        end

        def can_access_dev?(user, area)
          @dev_access_check.call(user, area)
        end

        def merge_requirements_met?(merge_request)
          merge_request[:approvals_count] >= @required_approvals
        end

        # Internal setters for DSL
        attr_writer :protect_main, :required_approvals, :reviewer_check,
                    :dev_access_check, :mr_create_check

        class PolicyDSL
          def initialize(policy)
            @policy = policy
          end

          def protect_main(value)
            @policy.protect_main = value
          end

          def required_approvals(count)
            @policy.required_approvals = count
          end

          def reviewer_check(&block)
            @policy.reviewer_check = block
          end

          def dev_access_check(&block)
            @policy.dev_access_check = block
          end

          def mr_create_check(&block)
            @policy.mr_create_check = block
          end
        end
      end
    end
  end
end
