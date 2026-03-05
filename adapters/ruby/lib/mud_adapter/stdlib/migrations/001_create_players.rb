Sequel.migration do # rubocop:disable Metrics/BlockLength
  change do # rubocop:disable Metrics/BlockLength
    create_table(:players) do
      String :id, primary_key: true
      String :password_hash, null: false
      column :ssh_keys, 'text[]', default: Sequel.lit("'{}'")
      String :active_character
      String :role, default: 'builder'
      Integer :builder_character_id
      column :created_at, 'timestamptz', default: Sequel::CURRENT_TIMESTAMP
    end

    create_table(:characters) do
      primary_key :id
      foreign_key :player_id, :players, type: String, null: false
      String :name, null: false
      column :created_at, 'timestamptz', default: Sequel::CURRENT_TIMESTAMP
      unique %i[player_id name]
    end

    alter_table(:players) do
      add_foreign_key [:builder_character_id], :characters
    end

    create_table(:access_tokens) do
      primary_key :id
      foreign_key :player_id, :players, type: String, null: false
      String :name, null: false
      String :token_prefix, null: false
      String :token_hash, null: false
      column :created_at, 'timestamptz', default: Sequel::CURRENT_TIMESTAMP
      column :last_used_at, 'timestamptz'
    end

    create_table(:sessions) do
      primary_key :id
      foreign_key :player_id, :players, type: String, null: false
      String :token, null: false, unique: true
      column :created_at, 'timestamptz', default: Sequel::CURRENT_TIMESTAMP
    end
  end
end
